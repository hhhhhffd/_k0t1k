//! Finding the files of blocked CurseForge mods (downloads disabled by the author) in a folder the
//! user saved them to, by SHA-1, and placing them where the pack wants them (Prism's BlockedModsDialog).

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use sha1::{Digest, Sha1};

use super::BlockedFile;
use super::modpack::safe_join;
use crate::error::{IoContext, Result};

/// Browsers write these while a download is running.
const PARTIAL_EXTENSIONS: [&str; 3] = ["crdownload", "part", "tmp"];
/// Bigger files are not mods; hashing them on every poll would be wasteful.
const MAX_CANDIDATE_BYTES: u64 = 512 * 1024 * 1024;

/// SHA-1 of a folder file by path, valid while its size and modification time are unchanged. Keep
/// one across polls so a folder is not re-read every time.
pub type HashCache = HashMap<PathBuf, (u64, SystemTime, String)>;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedScan {
    /// `target_path`s whose file is now in the game dir (just copied, or already there).
    pub found: Vec<String>,
    /// `target_path`s still missing.
    pub missing: Vec<String>,
}

fn sha1_of(path: &Path) -> Result<String> {
    let mut file = File::open(path).at(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).at(path)?;
        if read == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

/// Looks in `folder` (not recursively) for the files of `blocked` that are not in the game dir yet
/// and copies each match to its `target_path`. A file matches by SHA-1; one without a known hash
/// matches by name. Files in `folder` are never moved or deleted.
pub fn collect_blocked(game_dir: &Path, blocked: &[BlockedFile], folder: &Path, cache: &mut HashCache) -> Result<BlockedScan> {
    let mut scan = BlockedScan::default();
    let mut pending = Vec::new();
    for file in blocked {
        let target = safe_join(game_dir, &file.target_path)?;
        if target.exists() {
            scan.found.push(file.target_path.clone());
        } else {
            pending.push((file, target));
        }
    }
    if pending.is_empty() {
        return Ok(scan);
    }

    let mut by_hash: HashMap<String, PathBuf> = HashMap::new();
    let mut by_name: HashMap<String, PathBuf> = HashMap::new();
    for entry in std::fs::read_dir(folder).at(folder)? {
        let entry = entry.at(folder)?;
        let path = entry.path();
        let metadata = entry.metadata().at(&path)?;
        let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !metadata.is_file() || metadata.len() > MAX_CANDIDATE_BYTES || PARTIAL_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        by_name.insert(entry.file_name().to_string_lossy().to_lowercase(), path.clone());
        let modified = metadata.modified().at(&path)?;
        let hash = match cache.get(&path) {
            Some((size, time, hash)) if *size == metadata.len() && *time == modified => hash.clone(),
            _ => {
                let hash = sha1_of(&path)?;
                cache.insert(path.clone(), (metadata.len(), modified, hash.clone()));
                hash
            }
        };
        by_hash.insert(hash, path);
    }

    for (file, target) in pending {
        let source = match &file.sha1 {
            Some(sha1) => by_hash.get(&sha1.to_ascii_lowercase()),
            None => by_name.get(&file.file_name.to_lowercase()),
        };
        match source {
            Some(source) => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).at(parent)?;
                }
                std::fs::copy(source, &target).at(&target)?;
                scan.found.push(file.target_path.clone());
            }
            None => scan.missing.push(file.target_path.clone()),
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Platform;

    fn blocked(name: &str, content: Option<&str>) -> BlockedFile {
        BlockedFile {
            platform: Platform::CurseForge,
            project_id: "1".into(),
            name: name.into(),
            file_name: name.into(),
            target_path: format!("mods/{name}"),
            download_page: None,
            sha1: content.map(|content| hex::encode(Sha1::digest(content.as_bytes()))),
        }
    }

    #[test]
    fn matches_by_hash_under_any_name_and_reports_what_is_still_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (game_dir, downloads) = (dir.path().join("game"), dir.path().join("downloads"));
        std::fs::create_dir_all(game_dir.join("mods")).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        // The browser renamed it, and a partial download of another one is still running.
        std::fs::write(downloads.join("alpha (1).jar"), "alpha").unwrap();
        std::fs::write(downloads.join("beta.jar.crdownload"), "beta").unwrap();
        std::fs::write(downloads.join("unrelated.jar"), "other").unwrap();
        std::fs::write(game_dir.join("mods/placed.jar"), "placed").unwrap();

        let files = [
            blocked("alpha.jar", Some("alpha")),
            blocked("beta.jar", Some("beta")),
            blocked("placed.jar", Some("placed")),
            blocked("unhashed.jar", None),
        ];
        let mut cache = HashCache::new();
        let scan = collect_blocked(&game_dir, &files, &downloads, &mut cache).unwrap();
        assert_eq!(scan.found, ["mods/placed.jar", "mods/alpha.jar"]);
        assert_eq!(scan.missing, ["mods/beta.jar", "mods/unhashed.jar"]);
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/alpha.jar")).unwrap(), "alpha");
        assert!(downloads.join("alpha (1).jar").exists(), "the source is left in place");

        // The download finishes and the unhashed one shows up by name; the cache is reused.
        std::fs::rename(downloads.join("beta.jar.crdownload"), downloads.join("beta.jar")).unwrap();
        std::fs::write(downloads.join("Unhashed.jar"), "whatever").unwrap();
        let scan = collect_blocked(&game_dir, &files, &downloads, &mut cache).unwrap();
        assert_eq!(scan.found.len(), 4);
        assert!(scan.missing.is_empty());
    }

    #[test]
    fn a_target_path_that_escapes_the_game_dir_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = blocked("x.jar", None);
        file.target_path = "../x.jar".into();
        assert!(collect_blocked(dir.path(), &[file], dir.path(), &mut HashCache::new()).is_err());
    }
}
