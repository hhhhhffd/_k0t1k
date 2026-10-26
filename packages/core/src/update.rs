//! Launcher self-update from GitHub Releases: finds a newer release and downloads the new executable
//! after checking its Ed25519 signature. Swapping the running executable is the app's job.
//!
//! A release carries two assets: `_k0t1k.exe` and `_k0t1k.exe.sig`, the base64 of a raw 64-byte pure
//! Ed25519 signature over [`signed_message`]: a fixed prefix, the version and the SHA-256 of the exe
//! (`scripts/release.sh` makes it). Binding the version stops an old signed exe from being republished
//! under a newer tag. Only the public key is embedded here.
//!
//! GitHub's REST API is not used: unauthenticated it allows 60 requests an hour per IP, which a shared
//! network exhausts. The latest tag is read from the redirect of `github.com/<repo>/releases/latest`
//! and the assets are fetched from `releases/download/<tag>/`, neither of which is rate limited the
//! same way. While the repository is private or has no release the check reports [`UpdateCheck::Unavailable`].

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::LOCATION;
use serde::Serialize;
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
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Downloads are redirected from github.com to GitHub's asset hosts; a chain longer than this is not.
const MAX_REDIRECTS: usize = 5;

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("k0t1k-launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
}

/// HTTP client for update downloads, with timeouts. The shared client has none. It follows redirects
/// (release assets live on GitHub's asset hosts) but only over https and only a few of them.
pub fn http_client() -> reqwest::Client {
    client_builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                attempt.error("redirected to a location that is not https")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .expect("static reqwest client configuration is valid")
}

/// Whether a failed update step is a rate limit, a GitHub outage or a network problem, which resolve
/// themselves, rather than a rejected release.
pub fn is_transient(error: &Error) -> bool {
    match error {
        // No status: the request never got an answer (offline, DNS, connect, timeout, dropped body).
        Error::Http { source, .. } => source.status().is_none_or(|status| matches!(status.as_u16(), 403 | 429) || status.is_server_error()),
        _ => false,
    }
}

/// A release newer than the running launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    /// Normalised `X.Y.Z`, without the leading `v`.
    pub version: String,
    /// The release page on GitHub, where the release notes are.
    pub notes_url: String,
    /// Not sent to the UI.
    #[serde(skip)]
    pub exe_url: String,
    #[serde(skip)]
    pub sig_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum UpdateCheck {
    UpToDate,
    Available { release: ReleaseInfo },
    /// No release can be read: none published yet, or the repository is private.
    Unavailable,
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

/// Reads the tag from the `Location` of `github.com/<repo>/releases/latest`:
/// `https://github.com/<repo>/releases/tag/vX.Y.Z`. A repository without releases is redirected to
/// `.../releases`, which is `None`; any other location is refused.
fn version_from_location(location: &str, repo: &str) -> Result<Option<(u64, u64, u64)>> {
    let releases = format!("https://github.com/{repo}/releases");
    if location.eq_ignore_ascii_case(&releases) || location.eq_ignore_ascii_case(&format!("{releases}/")) {
        return Ok(None);
    }
    let tag_prefix = format!("{releases}/tag/");
    let tag = location.get(..tag_prefix.len()).filter(|head| head.eq_ignore_ascii_case(&tag_prefix)).map(|_| &location[tag_prefix.len()..]);
    // Asset URLs are rebuilt from the version, so the tag must be exactly `v` + a canonical version.
    tag.and_then(|tag| tag.strip_prefix('v').and_then(parse_version).filter(|version| normalized(*version) == tag[1..]))
        .map(Some)
        .ok_or_else(|| Error::Other(format!("unexpected release location {location:?}")))
}

/// The version of the latest release of `repo`, or `None` when it has none (or does not exist).
pub(crate) async fn latest_version(repo: &str) -> Result<Option<(u64, u64, u64)>> {
    latest_version_at(&format!("https://github.com/{repo}/releases/latest"), repo).await
}

async fn latest_version_at(url: &str, repo: &str) -> Result<Option<(u64, u64, u64)>> {
    let http = |source| Error::Http { url: url.to_owned(), source };
    // The redirect itself is the answer, so this request must not follow it.
    let client = client_builder().redirect(reqwest::redirect::Policy::none()).build().expect("static reqwest client configuration is valid");
    let response = client.get(url).timeout(REQUEST_TIMEOUT).send().await.map_err(http)?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_redirection() {
        return Err(match response.error_for_status() {
            Err(source) => http(source),
            Ok(_) => Error::Other(format!("unexpected answer {status} from {url}")),
        });
    }
    let location = response.headers().get(LOCATION).and_then(|value| value.to_str().ok()).unwrap_or_default();
    version_from_location(location, repo)
}

fn release_page(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

fn check_version(latest: Option<(u64, u64, u64)>, current: &str) -> Result<UpdateCheck> {
    let Some(latest) = latest else {
        return Ok(UpdateCheck::Unavailable);
    };
    let version = normalized(latest);
    if !is_newer(&version, current)? {
        return Ok(UpdateCheck::UpToDate);
    }
    let asset = |name: &str| format!("{ASSET_URL_PREFIX}v{version}/{name}");
    Ok(UpdateCheck::Available {
        release: ReleaseInfo { notes_url: release_page(&version), exe_url: asset(EXE_ASSET), sig_url: asset(SIG_ASSET), version },
    })
}

/// Asks GitHub for the latest release and compares it with `current`.
pub async fn check(current: &str) -> Result<UpdateCheck> {
    check_version(latest_version(REPO).await?, current)
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
    let response = client.get(url).timeout(REQUEST_TIMEOUT).send().await.map_err(http)?.error_for_status().map_err(http)?;
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

/// Downloads the release executable to `dest` and verifies it: the download must be complete and within
/// [`MAX_EXE_BYTES`], and the signature must cover this version and the SHA-256 of what was written.
/// On any failure `dest` is removed, so a file that exists after `Ok` is signed by the release key.
pub async fn download(client: &reqwest::Client, release: &ReleaseInfo, current: &str, dest: &Path, progress: &ProgressFn) -> Result<()> {
    let result = download_inner(client, release, current, dest, progress, PUBLIC_KEY, ASSET_URL_PREFIX).await;
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
    asset_prefix: &str,
) -> Result<()> {
    // Also guards a caller that did not come through `check`.
    if !is_newer(&release.version, current)? {
        return Err(Error::Other(format!("update {} is not newer than {current}", release.version)));
    }
    for url in [&release.exe_url, &release.sig_url] {
        if !url.starts_with(asset_prefix) {
            return Err(Error::Other(format!("unexpected update asset location {url}")));
        }
    }
    let signature = fetch_signature(client, &release.sig_url).await?;
    let url = release.exe_url.as_str();
    let http = |source| Error::Http { url: url.to_owned(), source };
    let response = client.get(url).send().await.map_err(http)?.error_for_status().map_err(http)?;
    // The signature is the integrity check; the length only catches a cut-off or absurd download early.
    let announced = response.content_length();
    if announced.is_some_and(|length| length == 0 || length > MAX_EXE_BYTES) {
        return Err(Error::Other(format!("unexpected update size {announced:?} bytes")));
    }
    let total = announced.unwrap_or(0);
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
        if done > MAX_EXE_BYTES || announced.is_some_and(|length| done > length) {
            return Err(Error::Other(format!("the update is larger than the {} bytes expected", announced.unwrap_or(MAX_EXE_BYTES))));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.at(dest)?;
        report(done);
    }
    if done == 0 || announced.is_some_and(|length| done != length) {
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

    const LOCATION_BASE: &str = "https://github.com/hhhhhffd/_k0t1k/releases";

    fn version_at(location: &str) -> Result<Option<(u64, u64, u64)>> {
        version_from_location(location, REPO)
    }

    #[test]
    fn location_with_a_tag_gives_the_version() {
        assert_eq!(version_at(&format!("{LOCATION_BASE}/tag/v0.1.1")).unwrap(), Some((0, 1, 1)));
        assert_eq!(version_at("https://GitHub.com/HHHHHFFD/_k0t1k/releases/tag/v10.20.30").unwrap(), Some((10, 20, 30)));
    }

    #[test]
    fn location_of_a_repository_without_releases_is_none() {
        assert_eq!(version_at(LOCATION_BASE).unwrap(), None);
        assert_eq!(version_at(&format!("{LOCATION_BASE}/")).unwrap(), None);
    }

    #[test]
    fn foreign_or_odd_locations_are_refused() {
        for location in [
            "",
            "https://evil.example/hhhhhffd/_k0t1k/releases/tag/v0.1.1",
            "http://github.com/hhhhhffd/_k0t1k/releases/tag/v0.1.1",
            "https://github.com/someone/else/releases/tag/v0.1.1",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/nightly",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/0.1.1",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/v0.1.1-beta",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/v01.1.1",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/v0.1.1/../../x",
            "https://github.com/hhhhhffd/_k0t1k/releases/tag/v0.1.1?x=1",
            "https://github.com/hhhhhffd/_k0t1k/releases/download/v0.1.1/_k0t1k.exe",
        ] {
            assert!(version_at(location).is_err(), "{location:?}");
        }
    }

    #[test]
    fn newer_release_is_available_with_rebuilt_urls() {
        let UpdateCheck::Available { release } = check_version(Some((0, 2, 0)), "0.1.0").unwrap() else { panic!("not available") };
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.notes_url, format!("{LOCATION_BASE}/tag/v0.2.0"));
        assert_eq!(release.exe_url, format!("{ASSET_URL_PREFIX}v0.2.0/_k0t1k.exe"));
        assert_eq!(release.sig_url, format!("{ASSET_URL_PREFIX}v0.2.0/_k0t1k.exe.sig"));
        let sent = serde_json::to_string(&release).unwrap();
        assert!(!sent.contains("download"), "{sent}");
    }

    #[test]
    fn same_older_and_missing_releases() {
        assert_eq!(check_version(Some((0, 1, 0)), "0.1.0").unwrap(), UpdateCheck::UpToDate);
        assert_eq!(check_version(Some((0, 0, 5)), "0.1.0").unwrap(), UpdateCheck::UpToDate);
        assert_eq!(check_version(None, "0.1.0").unwrap(), UpdateCheck::Unavailable);
        assert!(check_version(Some((0, 2, 0)), "nightly").is_err());
    }

    /// Answers every request with `status` and an optional `Location`, to test how answers are read.
    async fn serve_status(status: u16, location: Option<String>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 2048];
                let _ = socket.read(&mut request).await.unwrap();
                let location = location.as_ref().map(|value| format!("Location: {value}\r\n")).unwrap_or_default();
                let response = format!("HTTP/1.1 {status} X\r\n{location}Content-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        base
    }

    #[tokio::test]
    async fn latest_version_reads_the_redirect_without_following_it() {
        let found = serve_status(302, Some(format!("{LOCATION_BASE}/tag/v0.1.1"))).await;
        assert_eq!(latest_version_at(&found, REPO).await.unwrap(), Some((0, 1, 1)));
        let no_releases = serve_status(302, Some(LOCATION_BASE.to_owned())).await;
        assert_eq!(latest_version_at(&no_releases, REPO).await.unwrap(), None);
        let missing = serve_status(404, None).await;
        assert_eq!(latest_version_at(&missing, REPO).await.unwrap(), None);
        let foreign = serve_status(302, Some("https://evil.example/".to_owned())).await;
        let error = latest_version_at(&foreign, REPO).await.unwrap_err();
        assert!(!is_transient(&error));
        let ok_without_redirect = serve_status(200, None).await;
        assert!(latest_version_at(&ok_without_redirect, REPO).await.is_err());
    }

    #[tokio::test]
    async fn rate_limits_server_errors_and_network_failures_are_transient() {
        for (status, transient) in [(403, true), (429, true), (500, true), (503, true), (401, false), (410, false)] {
            let base = serve_status(status, None).await;
            let error = latest_version_at(&base, REPO).await.unwrap_err();
            assert_eq!(is_transient(&error), transient, "{status}");
        }
        // Nothing listens on port 1.
        let offline = latest_version_at("http://127.0.0.1:1/", REPO).await.unwrap_err();
        assert!(is_transient(&offline));
        // Rejected releases are real failures.
        assert!(!is_transient(&Error::Other("update signature check failed: x".into())));
        assert!(!is_transient(&Error::HashMismatch { url: String::new(), expected: String::new(), actual: String::new() }));
    }

    #[tokio::test]
    async fn downloads_do_not_follow_redirects_to_plain_http() {
        let target = serve_status(200, None).await;
        let redirect = serve_status(302, Some(format!("{target}/exe"))).await;
        assert!(http_client().get(&redirect).send().await.is_err());
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

    async fn try_download(exe: &[u8], sig: &[u8], version: &str, current: &str, prefix_ok: bool) -> (Result<()>, bool) {
        let base = serve(vec![("/exe", exe.to_vec()), ("/sig", sig.to_vec())]).await;
        let release = ReleaseInfo { version: version.into(), notes_url: String::new(), exe_url: format!("{base}/exe"), sig_url: format!("{base}/sig") };
        let prefix = if prefix_ok { base.as_str() } else { "https://github.com/" };
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("_k0t1k.exe.new");
        let result = download_inner(&reqwest::Client::new(), &release, current, &dest, &crate::progress::no_progress(), TEST_KEY, prefix).await;
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
    async fn download_accepts_only_the_signed_version_and_content_from_the_release_location() {
        let (ok, exists) = try_download(EXE, TEST_SIG.as_bytes(), "0.2.0", "0.1.0", true).await;
        ok.unwrap();
        assert!(exists);
        // Tampered content of the same length, wrong announced version, not newer, big signature, foreign location.
        for (exe, sig, version, current, prefix_ok) in [
            (b"k0t1k test exf".as_slice(), TEST_SIG.as_bytes(), "0.2.0", "0.1.0", true),
            (EXE, TEST_SIG.as_bytes(), "9.9.9", "0.1.0", true),
            (EXE, TEST_SIG.as_bytes(), "0.2.0", "0.2.0", true),
            (EXE, &[b'A'; 5000][..], "0.2.0", "0.1.0", true),
            (EXE, TEST_SIG.as_bytes(), "0.2.0", "0.1.0", false),
        ] {
            let (result, exists) = try_download(exe, sig, version, current, prefix_ok).await;
            assert!(result.is_err() && !exists, "{version} {current} {prefix_ok}");
        }
    }

    /// Live check against the real repository; run by hand: `cargo test -p k0t1k-core live_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_release_is_found_and_its_signature_verifies() {
        let UpdateCheck::Available { release } = check("0.1.0").await.unwrap() else { panic!("no update found") };
        println!("latest: {release:?}");
        assert_eq!(release.version, "0.1.1");
        assert_eq!(check("0.1.1").await.unwrap(), UpdateCheck::UpToDate);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("_k0t1k.exe.new");
        download(&http_client(), &release, "0.1.0", &dest, &crate::progress::no_progress()).await.unwrap();
        println!("downloaded and verified {} bytes", std::fs::metadata(&dest).unwrap().len());
    }
}
