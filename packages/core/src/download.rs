use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{StreamExt, TryStreamExt, stream};
use sha1::{Digest, Sha1};
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

pub async fn get_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
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

/// Download one file atomically (temp file + rename), verifying SHA-1 when known.
pub async fn download_file(client: &reqwest::Client, item: &DownloadItem) -> Result<()> {
    if let Some(expected) = &item.sha1
        && tokio::fs::try_exists(&item.path).await.at(&item.path)?
        && sha1_file(&item.path).await? == *expected
    {
        return Ok(());
    }

    if let Some(parent) = item.path.parent() {
        tokio::fs::create_dir_all(parent).await.at(parent)?;
    }

    let http = |source| Error::Http {
        url: item.url.clone(),
        source,
    };
    let response = client.get(&item.url).send().await.map_err(http)?;
    let mut body = response.error_for_status().map_err(http)?.bytes_stream();

    let tmp = item.path.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await.at(&tmp)?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(http)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.at(&tmp)?;
    }
    file.flush().await.at(&tmp)?;
    drop(file);

    if let Some(expected) = &item.sha1 {
        let actual = hex::encode(hasher.finalize());
        if actual != *expected {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(Error::HashMismatch {
                url: item.url.clone(),
                expected: expected.clone(),
                actual,
            });
        }
    }

    tokio::fs::rename(&tmp, &item.path).await.at(&item.path)
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
