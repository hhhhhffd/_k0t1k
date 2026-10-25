//! Launcher self-update from GitHub Releases: finds a newer release and downloads the new executable
//! after checking its Ed25519 signature. Swapping the running executable is the app's job.
//!
//! A release carries two assets: `_k0t1k.exe` and `_k0t1k.exe.sig`, the base64 of a raw 64-byte pure
//! Ed25519 signature over [`signed_message`]: a fixed prefix, the version and the SHA-256 of the exe
//! (`scripts/release.sh` makes it). Binding the version stops an old signed exe from being republished
//! under a newer tag. Only the public key is embedded here. No GitHub token is ever used, so while the repository is private the API answers 404
//! and the check reports [`UpdateCheck::Unavailable`].

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use futures::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};

const REPO: &str = "hhhhhffd/_k0t1k";
/// Raw 32-byte Ed25519 public key, base64. The private half is kept off the repository.
const PUBLIC_KEY: &str = "NrARzBayytnggNAo7TAAGOuAUyNb44vAMpDhECFZs4U=";
const EXE_ASSET: &str = "_k0t1k.exe";
const SIG_ASSET: &str = "_k0t1k.exe.sig";
/// Stable task id of the download, as shown in the UI's task list.
pub const TASK: &str = "update";
/// A signature is 88 base64 characters; anything much larger is not one.
const MAX_SIGNATURE_BYTES: usize = 1024;
/// The launcher is about 20 MB; a release asset far beyond that is not ours.
const MAX_EXE_BYTES: u64 = 200 * 1024 * 1024;
/// Assets are only accepted from this release download location.
const ASSET_URL_PREFIX: &str = concat!("https://github.com/", "hhhhhffd/_k0t1k", "/releases/download/");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest silence while reading a response; a stalled download fails instead of hanging.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const API_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP client for update requests, with timeouts. The shared client has none.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("k0t1k-launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("static reqwest client configuration is valid")
}

/// A release newer than the running launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    /// Normalised `X.Y.Z`, without the leading `v`.
    pub version: String,
    /// The release body as plain text.
    pub notes: String,
    /// Not sent to the UI.
    #[serde(skip)]
    pub exe_url: String,
    #[serde(skip)]
    pub sig_url: String,
    /// Exact size of the exe, from the release asset.
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum UpdateCheck {
    UpToDate,
    Available { release: ReleaseInfo },
    /// No release can be read: none published yet, or the repository is private.
    Unavailable,
}

#[derive(Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    pub(crate) draft: bool,
    #[serde(default)]
    pub(crate) prerelease: bool,
    #[serde(default)]
    pub(crate) assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
pub(crate) struct GithubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) size: u64,
}

/// `X.Y.Z` with an optional leading `v`; pre-release and build suffixes are not supported.
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut next = || {
        let part = parts.next()?;
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok()).flatten()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

pub(crate) fn normalized(version: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// Whether `candidate` is a valid version greater than `current`.
pub fn is_newer(candidate: &str, current: &str) -> Result<bool> {
    let parse = |text| parse_version(text).ok_or_else(|| Error::Other(format!("unsupported version {text:?}")));
    Ok(parse(candidate)? > parse(current)?)
}

fn check_release(release: GithubRelease, current: &str) -> Result<UpdateCheck> {
    if release.draft || release.prerelease || !is_newer(&release.tag_name, current)? {
        return Ok(UpdateCheck::UpToDate);
    }
    let asset = |name: &str| release.assets.iter().find(|asset| asset.name == name);
    let (Some(exe), Some(sig)) = (asset(EXE_ASSET), asset(SIG_ASSET)) else {
        // A release being assembled (or published without the helper script) is not installable.
        return Ok(UpdateCheck::Unavailable);
    };
    for url in [&exe.browser_download_url, &sig.browser_download_url] {
        if !url.starts_with(ASSET_URL_PREFIX) {
            return Err(Error::Other(format!("unexpected update asset location {url}")));
        }
    }
    if exe.size == 0 || exe.size > MAX_EXE_BYTES {
        return Err(Error::Other(format!("unexpected update size {} bytes", exe.size)));
    }
    let version = parse_version(&release.tag_name).ok_or_else(|| Error::Other(format!("unsupported version {:?}", release.tag_name)))?;
    Ok(UpdateCheck::Available {
        release: ReleaseInfo {
            version: normalized(version),
            notes: release.body.unwrap_or_default(),
            exe_url: exe.browser_download_url.clone(),
            sig_url: sig.browser_download_url.clone(),
            size: exe.size,
        },
    })
}

/// Asks GitHub for the latest release and compares it with `current`.
pub async fn check(client: &reqwest::Client, current: &str) -> Result<UpdateCheck> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let http = |source| Error::Http { url: url.clone(), source };
    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(API_TIMEOUT)
        .send()
        .await
        .map_err(http)?;
    // 404 is what a private repository or a repository without releases answers.
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(UpdateCheck::Unavailable);
    }
    let release = response.error_for_status().map_err(http)?.json().await.map_err(http)?;
    check_release(release, current)
}

/// The bytes the release key signs: binds the version and the content of the exe.
pub fn signed_message(version: &str, sha256: &[u8]) -> Vec<u8> {
    format!("_k0t1k-update\n{version}\n{}", hex::encode(sha256)).into_bytes()
}

/// Checks `signature_b64` (base64 of 64 raw bytes) over `message` against a base64 raw public key.
pub(crate) fn verify_with(public_key_b64: &str, message: &[u8], signature_b64: &str) -> Result<()> {
    let bad = |what: &str| Error::Other(format!("update signature check failed: {what}"));
    let key: [u8; 32] = STANDARD.decode(public_key_b64).ok().and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| bad("bad public key"))?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| bad("bad public key"))?;
    let signature: [u8; 64] =
        STANDARD.decode(signature_b64.trim()).ok().and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| bad("malformed signature"))?;
    key.verify_strict(message, &Signature::from_bytes(&signature)).map_err(|_| bad("the signature does not match"))
}

/// GETs `url` into memory, failing as soon as the body exceeds `max` bytes.
pub(crate) async fn fetch_capped(client: &reqwest::Client, url: &str, max: usize, what: &str) -> Result<Vec<u8>> {
    let http = |source| Error::Http { url: url.to_owned(), source };
    let response = client.get(url).timeout(API_TIMEOUT).send().await.map_err(http)?.error_for_status().map_err(http)?;
    let too_large = || Error::Other(format!("the {what} is too large"));
    if response.content_length().is_some_and(|length| length > max as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.map_err(http)?);
        if body.len() > max {
            return Err(too_large());
        }
    }
    Ok(body)
}

async fn fetch_signature(client: &reqwest::Client, url: &str) -> Result<String> {
    let body = fetch_capped(client, url, MAX_SIGNATURE_BYTES, "signature file").await?;
    String::from_utf8(body).map_err(|_| Error::Other("the signature file is not text".to_owned()))
}

/// Downloads the release executable to `dest` and verifies it: the size must match the release asset
/// exactly, and the signature must cover this version and the SHA-256 of what was written. On any
/// failure `dest` is removed, so a file that exists after `Ok` is signed by the release key.
pub async fn download(client: &reqwest::Client, release: &ReleaseInfo, current: &str, dest: &Path, progress: &ProgressFn) -> Result<()> {
    let result = download_inner(client, release, current, dest, progress, PUBLIC_KEY).await;
    if result.is_err()
        && let Err(error) = tokio::fs::remove_file(dest).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove the failed update download {}: {error}", dest.display());
    }
    result
}

async fn download_inner(
    client: &reqwest::Client,
    release: &ReleaseInfo,
    current: &str,
    dest: &Path,
    progress: &ProgressFn,
    public_key: &str,
) -> Result<()> {
    // Also guards a caller that did not come through `check`.
    if !is_newer(&release.version, current)? {
        return Err(Error::Other(format!("update {} is not newer than {current}", release.version)));
    }
    let total = release.size;
    if total == 0 || total > MAX_EXE_BYTES {
        return Err(Error::Other(format!("unexpected update size {total} bytes")));
    }
    let signature = fetch_signature(client, &release.sig_url).await?;
    let url = release.exe_url.as_str();
    let http = |source| Error::Http { url: url.to_owned(), source };
    let response = client.get(url).send().await.map_err(http)?.error_for_status().map_err(http)?;
    if response.content_length().is_some_and(|length| length != total) {
        return Err(Error::Other(format!("the update is {:?} bytes, the release says {total}", response.content_length())));
    }
    let stage = crate::i18n::tr("Downloading the update", "Скачивание обновления").to_owned();
    let report = |done| progress(ProgressEvent { task: TASK.to_owned(), stage: stage.clone(), done, total });

    let mut file = tokio::fs::File::create(dest).await.at(dest)?;
    let mut hash = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut done = 0u64;
    report(0);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(http)?;
        done += chunk.len() as u64;
        if done > total {
            return Err(Error::Other(format!("the update is larger than the {total} bytes the release says")));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.at(dest)?;
        report(done);
    }
    if done != total {
        return Err(Error::Other(format!("the update download ended at {done} of {total} bytes")));
    }
    // Flushed to disk before the file can take the exe's place.
    file.sync_all().await.at(dest)?;
    drop(file);
    verify_with(public_key, &signed_message(&release.version, &hash.finalize()), &signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway key pair made with openssl for these tests only. `TEST_SIG` signs
    // "_k0t1k-update\n0.2.0\n" + sha256(EXE) in hex.
    const TEST_KEY: &str = "BgpD+xo7gKQs5/LseJWCWR5Q9iO1WDI3cASFn82xL/s=";
    const TEST_SIG: &str = "3/gn5Ylpb6u88wVi2GlGrXMfYyM1qyPVKSTR7D96QjmE1Y3qw0nizp+Fx9DuOuWYuawQiLl8O5wDRjIuzkv7Ag==";
    const EXE: &[u8] = b"k0t1k test exe";

    fn message(version: &str, exe: &[u8]) -> Vec<u8> {
        signed_message(version, &Sha256::digest(exe))
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.1.1", "0.1.0").unwrap());
        assert!(is_newer("v0.10.0", "0.9.9").unwrap());
        assert!(is_newer("1.0.0", "0.99.99").unwrap());
        assert!(!is_newer("v0.1.0", "0.1.0").unwrap());
        assert!(!is_newer("v0.0.9", "0.1.0").unwrap());
        for bad in ["", "v1.2", "1.2.3.4", "1.2.x", "1.2.3-beta", "v", "1..3", "+1.2.3"] {
            assert!(is_newer(bad, "0.1.0").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn signed_message_format_is_stable() {
        assert_eq!(
            String::from_utf8(message("0.2.0", EXE)).unwrap(),
            "_k0t1k-update\n0.2.0\n77bc2441e6d5eb192fb0a14d90407beda4b5907069bc81d444e56f3da06b72a4"
        );
    }

    #[test]
    fn valid_signature_is_accepted() {
        verify_with(TEST_KEY, &message("0.2.0", EXE), TEST_SIG).unwrap();
        verify_with(TEST_KEY, &message("0.2.0", EXE), &format!("{TEST_SIG}\n")).unwrap();
    }

    #[test]
    fn other_version_content_or_signature_is_rejected() {
        // An old signed exe republished under a newer tag.
        assert!(verify_with(TEST_KEY, &message("9.9.9", EXE), TEST_SIG).is_err());
        assert!(verify_with(TEST_KEY, &message("0.2.0", b"k0t1k test exf"), TEST_SIG).is_err());
        assert!(verify_with(TEST_KEY, &message("0.2.0", b""), TEST_SIG).is_err());
        // The signature of the bare exe bytes (the old format) is not accepted.
        assert!(verify_with(TEST_KEY, EXE, TEST_SIG).is_err());
        let mut raw = STANDARD.decode(TEST_SIG).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        let good = message("0.2.0", EXE);
        assert!(verify_with(TEST_KEY, &good, &STANDARD.encode(&raw)).is_err());
        assert!(verify_with(TEST_KEY, &good, &STANDARD.encode(&raw[..63])).is_err());
        assert!(verify_with(TEST_KEY, &good, "!!!").is_err());
        assert!(verify_with(TEST_KEY, &good, "").is_err());
        // The embedded launcher key does not accept a signature of another key.
        assert!(verify_with(PUBLIC_KEY, &good, TEST_SIG).is_err());
    }

    #[test]
    fn embedded_public_key_is_a_valid_point() {
        let bytes: [u8; 32] = STANDARD.decode(PUBLIC_KEY).unwrap().try_into().unwrap();
        VerifyingKey::from_bytes(&bytes).unwrap();
    }

    fn release(json: &str) -> GithubRelease {
        serde_json::from_str(json).unwrap()
    }

    fn assets(size: u64) -> String {
        format!(
            r#""assets":[
            {{"name":"_k0t1k.exe","browser_download_url":"{ASSET_URL_PREFIX}v0.2.0/_k0t1k.exe","size":{size}}},
            {{"name":"_k0t1k.exe.sig","browser_download_url":"{ASSET_URL_PREFIX}v0.2.0/_k0t1k.exe.sig","size":88}}]"#
        )
    }

    #[test]
    fn newer_release_with_both_assets_is_available() {
        let json = format!(r#"{{"tag_name":"v0.2.0","body":"Fixes\r\n- a","draft":false,"prerelease":false,{}}}"#, assets(7));
        let UpdateCheck::Available { release } = check_release(release(&json), "0.1.0").unwrap() else { panic!("not available") };
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.notes, "Fixes\r\n- a");
        assert_eq!(release.exe_url, format!("{ASSET_URL_PREFIX}v0.2.0/_k0t1k.exe"));
        assert_eq!(release.size, 7);
        let sent = serde_json::to_string(&release).unwrap();
        assert!(!sent.contains("http"), "{sent}");
    }

    #[test]
    fn same_older_draft_and_prerelease_are_up_to_date() {
        for json in [
            format!(r#"{{"tag_name":"v0.1.0",{}}}"#, assets(7)),
            format!(r#"{{"tag_name":"v0.0.5",{}}}"#, assets(7)),
            format!(r#"{{"tag_name":"v0.3.0","draft":true,{}}}"#, assets(7)),
            format!(r#"{{"tag_name":"v0.3.0","prerelease":true,{}}}"#, assets(7)),
        ] {
            assert_eq!(check_release(release(&json), "0.1.0").unwrap(), UpdateCheck::UpToDate, "{json}");
        }
    }

    #[test]
    fn release_without_signature_asset_or_with_odd_tag() {
        let no_sig = r#"{"tag_name":"v0.2.0","body":null,"assets":[{"name":"_k0t1k.exe","browser_download_url":"u","size":1}]}"#;
        assert_eq!(check_release(release(no_sig), "0.1.0").unwrap(), UpdateCheck::Unavailable);
        assert!(check_release(release(r#"{"tag_name":"nightly","assets":[]}"#), "0.1.0").is_err());
    }

    #[test]
    fn foreign_asset_hosts_and_absurd_sizes_are_refused() {
        let foreign = format!(r#"{{"tag_name":"v0.2.0",{}}}"#, assets(7)).replace("https://github.com/", "http://evil.example/");
        assert!(check_release(release(&foreign), "0.1.0").is_err());
        let other_repo = format!(r#"{{"tag_name":"v0.2.0",{}}}"#, assets(7)).replace("hhhhhffd", "someone");
        assert!(check_release(release(&other_repo), "0.1.0").is_err());
        for size in [0, MAX_EXE_BYTES + 1] {
            let json = format!(r#"{{"tag_name":"v0.2.0",{}}}"#, assets(size));
            assert!(check_release(release(&json), "0.1.0").is_err(), "{size}");
        }
    }

    /// Serves `routes` (path to body) over plain HTTP on localhost; enough for the download code.
    async fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let read = socket.read(&mut request).await.unwrap();
                let line = String::from_utf8_lossy(&request[..read]).lines().next().unwrap_or_default().to_owned();
                let body = routes.iter().find(|(path, _)| line.contains(&format!(" {path} "))).map(|(_, body)| body);
                let head = if body.is_some() { "200 OK" } else { "404 Not Found" };
                let body = body.cloned().unwrap_or_default();
                let response = format!("HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        base
    }

    async fn try_download(exe: &[u8], sig: &[u8], size: u64, version: &str, current: &str) -> (Result<()>, bool) {
        let base = serve(vec![("/exe", exe.to_vec()), ("/sig", sig.to_vec())]).await;
        let release =
            ReleaseInfo { version: version.into(), notes: String::new(), exe_url: format!("{base}/exe"), sig_url: format!("{base}/sig"), size };
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("_k0t1k.exe.new");
        let result = download_inner(&reqwest::Client::new(), &release, current, &dest, &crate::progress::no_progress(), TEST_KEY).await;
        let result = match result {
            Ok(()) => Ok(()),
            err => {
                // `download` removes the file on failure; do what it does.
                let _ = std::fs::remove_file(&dest);
                err
            }
        };
        (result, dest.exists())
    }

    #[tokio::test]
    async fn download_accepts_only_the_signed_version_size_and_content() {
        let size = EXE.len() as u64;
        let (ok, exists) = try_download(EXE, TEST_SIG.as_bytes(), size, "0.2.0", "0.1.0").await;
        ok.unwrap();
        assert!(exists);
        // Tampered content of the right size, wrong announced version, not newer, wrong sizes, big signature.
        for (exe, sig, size, version, current) in [
            (b"k0t1k test exf".as_slice(), TEST_SIG.as_bytes(), size, "0.2.0", "0.1.0"),
            (EXE, TEST_SIG.as_bytes(), size, "9.9.9", "0.1.0"),
            (EXE, TEST_SIG.as_bytes(), size, "0.2.0", "0.2.0"),
            (EXE, TEST_SIG.as_bytes(), size + 1, "0.2.0", "0.1.0"),
            (EXE, TEST_SIG.as_bytes(), size - 1, "0.2.0", "0.1.0"),
            (EXE, &[b'A'; 5000][..], size, "0.2.0", "0.1.0"),
        ] {
            let (result, exists) = try_download(exe, sig, size, version, current).await;
            assert!(result.is_err() && !exists, "{version} {current} {size}");
        }
    }
}
