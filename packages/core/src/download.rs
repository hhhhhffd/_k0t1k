use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{StreamExt, TryStreamExt, stream};
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};

const USER_AGENT: &str = concat!("k0t1k-launcher/", env!("CARGO_PKG_VERSION"));
const PARALLEL_DOWNLOADS: usize = 16;

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("static reqwest client configuration is valid")
}

#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub url: String,
    pub path: PathBuf,
    /// Lowercase hex SHA-1. When present, an existing file with the same hash is not re-downloaded.
    pub sha1: Option<String>,
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T> {
    let http = |source| Error::Http {
        url: url.to_owned(),
        source,
    };
    let response = client.get(url).send().await.map_err(http)?;
    let response = response.error_for_status().map_err(http)?;
    response.json().await.map_err(http)
}

pub async fn sha1_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path).await.at(path)?;
    Ok(hex::encode(Sha1::digest(&bytes)))
}

/// Expected digest of a downloaded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    /// Lowercase hex SHA-1.
    Sha1(String),
    /// Lowercase hex SHA-256.
    Sha256(String),
    /// Lowercase hex SHA-512.
    Sha512(String),
}

impl Checksum {
    fn expected(&self) -> &str {
        match self {
            Checksum::Sha1(hex) | Checksum::Sha256(hex) | Checksum::Sha512(hex) => hex,
        }
    }

    fn hasher(&self) -> Hasher {
        match self {
            Checksum::Sha1(_) => Hasher::Sha1(Sha1::new()),
            Checksum::Sha256(_) => Hasher::Sha256(Sha256::new()),
            Checksum::Sha512(_) => Hasher::Sha512(Sha512::new()),
        }
    }
}

enum Hasher {
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Sha1(hasher) => hasher.update(data),
            Hasher::Sha256(hasher) => hasher.update(data),
            Hasher::Sha512(hasher) => hasher.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Sha1(hasher) => hex::encode(hasher.finalize()),
            Hasher::Sha256(hasher) => hex::encode(hasher.finalize()),
            Hasher::Sha512(hasher) => hex::encode(hasher.finalize()),
        }
    }
}

async fn file_matches(path: &Path, checksum: &Checksum) -> Result<bool> {
    if !tokio::fs::try_exists(path).await.at(path)? {
        return Ok(false);
    }
    let bytes = tokio::fs::read(path).await.at(path)?;
    let mut hasher = checksum.hasher();
    hasher.update(&bytes);
    Ok(hasher.finish() == checksum.expected())
}

/// Download one file atomically (temp file + rename), verifying SHA-1 when known.
pub async fn download_file(client: &reqwest::Client, item: &DownloadItem) -> Result<()> {
    let checksum = item.sha1.clone().map(Checksum::Sha1);
    download_checked(client, &item.url, &item.path, checksum.as_ref()).await
}

/// Download one file atomically, verifying `checksum` when known. An existing file that already
/// matches the checksum is kept.
pub async fn download_checked(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    checksum: Option<&Checksum>,
) -> Result<()> {
    if let Some(checksum) = checksum
        && file_matches(path, checksum).await?
    {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.at(parent)?;
    }

    let http = |source| Error::Http {
        url: url.to_owned(),
        source,
    };
    let response = client.get(url).send().await.map_err(http)?;
    let body = response.error_for_status().map_err(http)?.bytes_stream();

    // Unique per download: the same file may be fetched by several installs at once.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.part", uuid::Uuid::new_v4().simple()));
    let tmp = PathBuf::from(tmp);
    let written = write_body(body, &tmp, checksum, url).await;
    if let Err(error) = written {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    place_file(&tmp, path, checksum).await
}

async fn write_body<B: AsRef<[u8]>>(
    mut body: impl futures::Stream<Item = reqwest::Result<B>> + Unpin,
    tmp: &Path,
    checksum: Option<&Checksum>,
    url: &str,
) -> Result<()> {
    let http = |source| Error::Http {
        url: url.to_owned(),
        source,
    };
    let mut file = tokio::fs::File::create(tmp).await.at(tmp)?;
    let mut hasher = checksum.map(Checksum::hasher);
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(http)?;
        if let Some(hasher) = &mut hasher {
            hasher.update(chunk.as_ref());
        }
        file.write_all(chunk.as_ref()).await.at(tmp)?;
    }
    file.flush().await.at(tmp)?;

    if let (Some(checksum), Some(hasher)) = (checksum, hasher) {
        let actual = hasher.finish();
        if actual != checksum.expected() {
            return Err(Error::HashMismatch {
                url: url.to_owned(),
                expected: checksum.expected().to_owned(),
                actual,
            });
        }
    }
    Ok(())
}

/// Moves the finished temp file into place. When the rename fails because a concurrent download
/// already put the file there (Windows cannot replace a file that is open), the existing file is
/// accepted if it matches `checksum`.
async fn place_file(tmp: &Path, path: &Path, checksum: Option<&Checksum>) -> Result<()> {
    let renamed = tokio::fs::rename(tmp, path).await;
    let Err(error) = renamed else {
        return Ok(());
    };
    let _ = tokio::fs::remove_file(tmp).await;
    if let Some(checksum) = checksum
        && file_matches(path, checksum).await.unwrap_or(false)
    {
        return Ok(());
    }
    Err(error).at(path)
}

/// Download many files in parallel, reporting progress per finished file.
/// Fails on the first error; already finished files stay on disk and are reused next time.
pub async fn download_all(
    client: &reqwest::Client,
    items: Vec<DownloadItem>,
    task: &str,
    stage: &str,
    progress: &ProgressFn,
) -> Result<()> {
    let total = items.len() as u64;
    let done = Arc::new(AtomicU64::new(0));
    let report = |done: u64| {
        progress(ProgressEvent {
            task: task.to_owned(),
            stage: stage.to_owned(),
            done,
            total,
        })
    };
    report(0);

    stream::iter(items)
        .map(|item| {
            let done = done.clone();
            async move {
                download_file(client, &item).await?;
                report(done.fetch_add(1, Ordering::Relaxed) + 1);
                Ok::<_, Error>(())
            }
        })
        .buffer_unordered(PARALLEL_DOWNLOADS)
        .try_collect::<()>()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha1_of(data: &[u8]) -> Checksum {
        Checksum::Sha1(hex::encode(Sha1::digest(data)))
    }

    #[tokio::test]
    async fn write_body_verifies_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("a.part");
        let chunks = || stream::iter([Ok::<_, reqwest::Error>(b"he".to_vec()), Ok(b"llo".to_vec())]);

        write_body(chunks(), &tmp, Some(&sha1_of(b"hello")), "u").await.unwrap();
        assert_eq!(std::fs::read(&tmp).unwrap(), b"hello");
        let error = write_body(chunks(), &tmp, Some(&sha1_of(b"other")), "u").await.unwrap_err();
        assert!(matches!(error, Error::HashMismatch { .. }));
    }

    #[tokio::test]
    async fn place_file_replaces_or_fails() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("a.part");
        // A directory cannot be replaced by a file, which stands in for a failing rename.
        let target = dir.path().join("a");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&tmp, b"hello").unwrap();
        assert!(place_file(&tmp, &target, Some(&sha1_of(b"hello"))).await.is_err());
        assert!(!tmp.exists());

        // A normal rename replaces the existing file.
        std::fs::remove_dir(&target).unwrap();
        std::fs::write(&target, b"hello").unwrap();
        std::fs::write(&tmp, b"hello").unwrap();
        place_file(&tmp, &target, Some(&sha1_of(b"hello"))).await.unwrap();
    }
}
