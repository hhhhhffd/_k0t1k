//! Results of the RatScan plugin, kept per instance in `<instance>/ratscan.json` and keyed by the
//! SHA-1 of the jar together with the plugin version that produced them, so an unchanged jar is not
//! scanned again and a newer scanner rescans everything.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::error::{IoContext, Result};

const FILE: &str = "ratscan.json";
/// Oldest entries go first when a cache grows beyond this (jars come and go over the years).
const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanEntry {
    pub plugin_version: String,
    pub scanned_at: DateTime<Utc>,
    /// What the plugin returned: `{summary, report}`.
    pub result: Value,
}

pub type ScanCache = BTreeMap<String, ScanEntry>;

fn cache_file(instance_dir: &Path) -> PathBuf {
    instance_dir.join(FILE)
}

/// Lowercase hex SHA-1 of a file, read in chunks.
pub fn sha1_of(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).at(path)?;
    let mut hash = Sha1::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).at(path)?;
        if read == 0 {
            return Ok(hex::encode(hash.finalize()));
        }
        hash.update(&buffer[..read]);
    }
}

/// A missing or unreadable cache is empty (and logged): jars are simply scanned again.
pub fn load(instance_dir: &Path) -> ScanCache {
    let file = cache_file(instance_dir);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!("ignoring {}: {error}", file.display());
            ScanCache::new()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ScanCache::new(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", file.display());
            ScanCache::new()
        }
    }
}

/// Adds or replaces the entry of `sha1`. The caller serialises calls per instance.
pub fn store(instance_dir: &Path, sha1: &str, entry: ScanEntry) -> Result<()> {
    let mut cache = load(instance_dir);
    cache.insert(sha1.to_owned(), entry);
    while cache.len() > MAX_ENTRIES {
        let Some(oldest) = cache.iter().min_by_key(|(_, entry)| entry.scanned_at).map(|(key, _)| key.clone()) else {
            break;
        };
        cache.remove(&oldest);
    }
    let (file, pending) = (cache_file(instance_dir), instance_dir.join("ratscan.json.tmp"));
    std::fs::write(&pending, serde_json::to_vec(&cache)?).at(&pending)?;
    std::fs::rename(&pending, &file).at(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str, at: i64) -> ScanEntry {
        ScanEntry {
            plugin_version: version.to_owned(),
            scanned_at: DateTime::from_timestamp(at, 0).unwrap(),
            result: serde_json::json!({"summary": "ok", "report": {"verdict": "clean"}}),
        }
    }

    #[test]
    fn entries_round_trip_and_hash_is_sha1() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_empty());
        store(dir.path(), "aa", entry("0.1.0", 1)).unwrap();
        store(dir.path(), "bb", entry("0.1.0", 2)).unwrap();
        assert_eq!(load(dir.path()).len(), 2);
        assert_eq!(load(dir.path())["aa"], entry("0.1.0", 1));
        std::fs::write(dir.path().join("a.bin"), b"hello").unwrap();
        assert_eq!(sha1_of(&dir.path().join("a.bin")).unwrap(), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }

    #[test]
    fn a_broken_cache_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ratscan.json"), "{").unwrap();
        assert!(load(dir.path()).is_empty());
        store(dir.path(), "aa", entry("0.1.0", 1)).unwrap();
        assert_eq!(load(dir.path()).len(), 1);
    }
}
