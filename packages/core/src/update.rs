//! Launcher self-update from GitHub Releases: finds a newer release and downloads the new executable
//! after checking its Ed25519 signature. Swapping the running executable is the app's job.
//!
//! A release carries two assets: `_k0t1k.exe` and `_k0t1k.exe.sig`, the base64 of a raw 64-byte pure
//! Ed25519 signature over the executable's bytes (`scripts/release.sh` makes it). Only the public key is
//! embedded here. No GitHub token is ever used, so while the repository is private the API answers 404
//! and the check reports [`UpdateCheck::Unavailable`].

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use futures::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
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

/// A release newer than the running launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    /// Without the leading `v`.
    pub version: String,
    /// The release body as plain text.
    pub notes: String,
    pub exe_url: String,
    pub sig_url: String,
    pub size: Option<u64>,
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
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// `X.Y.Z` with an optional leading `v`; pre-release and build suffixes are not supported.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut next = || {
        let part = parts.next()?;
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok()).flatten()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
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
    Ok(UpdateCheck::Available {
        release: ReleaseInfo {
            version: release.tag_name.trim_start_matches('v').to_owned(),
            notes: release.body.unwrap_or_default(),
            exe_url: exe.browser_download_url.clone(),
            sig_url: sig.browser_download_url.clone(),
            size: Some(exe.size),
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

/// Checks `signature_b64` (base64 of 64 raw bytes) over `data` against a base64 raw public key.
fn verify_with(public_key_b64: &str, data: &[u8], signature_b64: &str) -> Result<()> {
    let bad = |what: &str| Error::Other(format!("update signature check failed: {what}"));
    let key: [u8; 32] = STANDARD.decode(public_key_b64).ok().and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| bad("bad public key"))?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| bad("bad public key"))?;
    let signature: [u8; 64] =
        STANDARD.decode(signature_b64.trim()).ok().and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| bad("malformed signature"))?;
    key.verify_strict(data, &Signature::from_bytes(&signature)).map_err(|_| bad("the signature does not match"))
}

/// Verifies `data` against the launcher's embedded public key.
pub fn verify(data: &[u8], signature_b64: &str) -> Result<()> {
    verify_with(PUBLIC_KEY, data, signature_b64)
}

async fn fetch_signature(client: &reqwest::Client, url: &str) -> Result<String> {
    let http = |source| Error::Http { url: url.to_owned(), source };
    let response = client.get(url).send().await.map_err(http)?.error_for_status().map_err(http)?;
    let bytes = response.bytes().await.map_err(http)?;
    if bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(Error::Other("the signature file is too large".to_owned()));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::Other("the signature file is not text".to_owned()))
}

/// Downloads the release executable to `dest` and verifies it. On any failure `dest` is removed, so
/// a file that exists after `Ok` is signed by the release key.
pub async fn download(client: &reqwest::Client, release: &ReleaseInfo, dest: &Path, progress: &ProgressFn) -> Result<()> {
    let result = download_inner(client, release, dest, progress).await;
    if result.is_err()
        && let Err(error) = tokio::fs::remove_file(dest).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove the failed update download {}: {error}", dest.display());
    }
    result
}

async fn download_inner(client: &reqwest::Client, release: &ReleaseInfo, dest: &Path, progress: &ProgressFn) -> Result<()> {
    let signature = fetch_signature(client, &release.sig_url).await?;
    let url = release.exe_url.as_str();
    let http = |source| Error::Http { url: url.to_owned(), source };
    let response = client.get(url).send().await.map_err(http)?.error_for_status().map_err(http)?;
    let total = response.content_length().or(release.size).unwrap_or(0);
    let stage = crate::i18n::tr("Downloading the update", "Скачивание обновления").to_owned();
    let report = |done| progress(ProgressEvent { task: TASK.to_owned(), stage: stage.clone(), done, total });

    let mut file = tokio::fs::File::create(dest).await.at(dest)?;
    let mut stream = response.bytes_stream();
    let mut done = 0u64;
    report(0);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(http)?;
        file.write_all(&chunk).await.at(dest)?;
        done += chunk.len() as u64;
        report(done);
    }
    file.flush().await.at(dest)?;
    drop(file);

    let bytes = tokio::fs::read(dest).await.at(dest)?;
    verify(&bytes, &signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway key pair made with openssl for these tests only; it signs `EXE`.
    const TEST_KEY: &str = "BgpD+xo7gKQs5/LseJWCWR5Q9iO1WDI3cASFn82xL/s=";
    const TEST_SIG: &str = "jox+uwZvLRG+LYSI6qKFixah7uNU45n4/pMwOXYf4lEJTdHwsjjP6+myQy0Dy2sFVFLgXzr1pkVYSdhkjrDlBg==";
    const EXE: &[u8] = b"k0t1k test exe";

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
    fn valid_signature_is_accepted() {
        verify_with(TEST_KEY, EXE, TEST_SIG).unwrap();
        verify_with(TEST_KEY, EXE, &format!("{TEST_SIG}\n")).unwrap();
    }

    #[test]
    fn tampered_data_or_bad_signature_is_rejected() {
        assert!(verify_with(TEST_KEY, b"k0t1k test exf", TEST_SIG).is_err());
        assert!(verify_with(TEST_KEY, b"", TEST_SIG).is_err());
        // Flipped last signature byte.
        let mut raw = STANDARD.decode(TEST_SIG).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        assert!(verify_with(TEST_KEY, EXE, &STANDARD.encode(&raw)).is_err());
        // Truncated, not base64, empty.
        assert!(verify_with(TEST_KEY, EXE, &STANDARD.encode(&raw[..63])).is_err());
        assert!(verify_with(TEST_KEY, EXE, "!!!").is_err());
        assert!(verify_with(TEST_KEY, EXE, "").is_err());
        // The embedded launcher key does not accept a signature of another key.
        assert!(verify(EXE, TEST_SIG).is_err());
    }

    #[test]
    fn embedded_public_key_is_a_valid_point() {
        let bytes: [u8; 32] = STANDARD.decode(PUBLIC_KEY).unwrap().try_into().unwrap();
        VerifyingKey::from_bytes(&bytes).unwrap();
    }

    fn release(json: &str) -> GithubRelease {
        serde_json::from_str(json).unwrap()
    }

    const ASSETS: &str = r#""assets":[
        {"name":"_k0t1k.exe","browser_download_url":"https://example.invalid/exe","size":7},
        {"name":"_k0t1k.exe.sig","browser_download_url":"https://example.invalid/sig","size":88}]"#;

    #[test]
    fn newer_release_with_both_assets_is_available() {
        let json = format!(r#"{{"tag_name":"v0.2.0","body":"Fixes\r\n- a","draft":false,"prerelease":false,{ASSETS}}}"#);
        let UpdateCheck::Available { release } = check_release(release(&json), "0.1.0").unwrap() else { panic!("not available") };
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.notes, "Fixes\r\n- a");
        assert_eq!(release.exe_url, "https://example.invalid/exe");
        assert_eq!(release.sig_url, "https://example.invalid/sig");
        assert_eq!(release.size, Some(7));
    }

    #[test]
    fn same_older_draft_and_prerelease_are_up_to_date() {
        for json in [
            format!(r#"{{"tag_name":"v0.1.0",{ASSETS}}}"#),
            format!(r#"{{"tag_name":"v0.0.5",{ASSETS}}}"#),
            format!(r#"{{"tag_name":"v0.3.0","draft":true,{ASSETS}}}"#),
            format!(r#"{{"tag_name":"v0.3.0","prerelease":true,{ASSETS}}}"#),
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
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        base
    }

    #[tokio::test]
    async fn download_keeps_a_verified_file_and_removes_a_bad_one() {
        // The embedded key cannot verify the throwaway signature, so only the failure path is reachable
        // here; the success path of `verify_with` is covered above.
        let base = serve(vec![("/exe", EXE.to_vec()), ("/sig", TEST_SIG.as_bytes().to_vec())]).await;
        let release = ReleaseInfo {
            version: "0.2.0".into(),
            notes: String::new(),
            exe_url: format!("{base}/exe"),
            sig_url: format!("{base}/sig"),
            size: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("_k0t1k.exe.new");
        let error = download(&reqwest::Client::new(), &release, &dest, &crate::progress::no_progress()).await.unwrap_err();
        assert!(error.to_string().contains("signature"), "{error}");
        assert!(!dest.exists());

        let missing = ReleaseInfo { sig_url: format!("{base}/nope"), ..release };
        assert!(download(&reqwest::Client::new(), &missing, &dest, &crate::progress::no_progress()).await.is_err());
        assert!(!dest.exists());
    }
}
