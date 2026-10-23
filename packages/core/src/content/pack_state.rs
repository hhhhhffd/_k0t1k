//! What a modpack installed into an instance, kept in `<instance>/pack-state.json` (Prism keeps the
//! same for mrpack in `mrpack/overrides.txt`). An update diffs it against the new pack, so it removes
//! exactly the files and overrides the old pack brought and never touches what the user added.
//! Also the pre-update backup that makes a failed update reversible.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::modpack::safe_join;
use crate::error::{Error, IoContext, Result};
use crate::instance::exists_in_linked_shared_folder;

pub const PACK_STATE_FILE: &str = "pack-state.json";
const BACKUP_DIR: &str = "backups";
const BACKUP_SUFFIX: &str = "-modpack-update.zip";
const KEPT_BACKUPS: usize = 3;
const GAME_DIR_PREFIX: &str = "minecraft/";

/// A file the pack downloads. `id` changes when the file's content does: the sha512 for mrpack,
/// the file id for CurseForge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFileState {
    pub path: String,
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PackState {
    /// Downloaded files, paths relative to the game dir with `/` separators.
    pub files: Vec<PackFileState>,
    /// Files the pack unpacked from its override folders, relative to the game dir.
    pub overrides: Vec<String>,
}

/// What an update has to do, by comparing the installed state with the new pack.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PackDiff {
    /// Old pack files that the new pack no longer has.
    pub removed_files: Vec<String>,
    /// Old override files that the new pack no longer has.
    pub removed_overrides: Vec<String>,
}

impl PackState {
    pub fn load(instance_dir: &Path) -> Result<Self> {
        let path = instance_dir.join(PACK_STATE_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).at(&path),
        }
    }

    pub fn save(&self, instance_dir: &Path) -> Result<()> {
        let path = instance_dir.join(PACK_STATE_FILE);
        std::fs::write(&path, serde_json::to_vec_pretty(self)?).at(&path)
    }

    /// True when the pack installed `path` with the same content id.
    pub fn has_file(&self, path: &str, id: &str) -> bool {
        self.files.iter().any(|file| file.path == path && file.id == id)
    }

    pub fn diff(&self, new_files: &[PackFileState], new_overrides: &[String]) -> PackDiff {
        let kept: BTreeSet<&str> = new_files
            .iter()
            .map(|file| file.path.as_str())
            .chain(new_overrides.iter().map(String::as_str))
            .collect();
        let gone = |path: &&str| !kept.contains(path);
        PackDiff {
            removed_files: self
                .files
                .iter()
                .map(|file| file.path.as_str())
                .filter(gone)
                .map(str::to_owned)
                .collect(),
            removed_overrides: self.overrides.iter().map(String::as_str).filter(gone).map(str::to_owned).collect(),
        }
    }
}

/// A zip of everything an update may change, taken before it starts. `restore` puts it all back.
pub struct Backup {
    zip_path: PathBuf,
    game_dir: PathBuf,
    instance_dir: PathBuf,
    dirs: Vec<String>,
    files: Vec<String>,
}

impl Backup {
    /// Saves the `dirs` (whole folders, e.g. `mods`) and the `files` that exist, as `minecraft/<path>`,
    /// plus `pack-state.json`. Files in linked shared folders are left out: they are not the pack's.
    pub fn create(instance_dir: &Path, game_dir: &Path, dirs: &[&str], files: &[String]) -> Result<Self> {
        let backups = instance_dir.join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).at(&backups)?;
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let zip_path = backups.join(format!("{stamp}{BACKUP_SUFFIX}"));
        let file = File::create(&zip_path).at(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);
        // Jars are compressed already; storing them keeps the backup of a big mods folder fast.
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        // A path can be listed twice (inside a saved folder and on its own); the map keeps one.
        let mut sources: BTreeMap<String, PathBuf> = BTreeMap::new();
        for dir in dirs {
            collect(&game_dir.join(dir), dir, &mut sources)?;
        }
        for path in files.iter().filter(|path| !exists_in_linked_shared_folder(game_dir, &game_dir.join(path))) {
            let absolute = safe_join(game_dir, path)?;
            if absolute.is_file() {
                sources.insert(path.clone(), absolute);
            }
        }
        let state_file = instance_dir.join(PACK_STATE_FILE);
        for (relative, absolute) in &sources {
            copy_into_zip(&mut writer, options, &format!("{GAME_DIR_PREFIX}{relative}"), absolute)?;
        }
        if state_file.is_file() {
            copy_into_zip(&mut writer, options, PACK_STATE_FILE, &state_file)?;
        }
        writer.finish()?;
        prune(&backups)?;
        Ok(Self {
            zip_path,
            game_dir: game_dir.to_owned(),
            instance_dir: instance_dir.to_owned(),
            dirs: dirs.iter().map(|dir| (*dir).to_owned()).collect(),
            files: files.to_vec(),
        })
    }

    /// Deletes what the update may have created or changed, then extracts the backup.
    pub fn restore(&self) -> Result<()> {
        for dir in &self.dirs {
            let path = self.game_dir.join(dir);
            if path.exists() {
                std::fs::remove_dir_all(&path).at(&path)?;
            }
        }
        for path in &self.files {
            let absolute = safe_join(&self.game_dir, path)?;
            if !exists_in_linked_shared_folder(&self.game_dir, &absolute) && absolute.is_file() {
                std::fs::remove_file(&absolute).at(&absolute)?;
            }
        }
        let mut archive = zip::ZipArchive::new(File::open(&self.zip_path).at(&self.zip_path)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name()?.into_owned();
            let destination = match name.strip_prefix(GAME_DIR_PREFIX) {
                Some(relative) => safe_join(&self.game_dir, relative)?,
                None if name == PACK_STATE_FILE => self.instance_dir.join(PACK_STATE_FILE),
                None => return Err(Error::Other(format!("unexpected entry {name:?} in the backup"))),
            };
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            let mut out = File::create(&destination).at(&destination)?;
            std::io::copy(&mut entry, &mut out).at(&destination)?;
        }
        Ok(())
    }
}

fn collect(dir: &Path, relative: &str, out: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).at(dir),
    };
    for entry in entries {
        let entry = entry.at(dir)?;
        let path = entry.path();
        let child = format!("{relative}/{}", entry.file_name().to_string_lossy());
        if entry.file_type().at(&path)?.is_dir() {
            collect(&path, &child, out)?;
        } else {
            out.insert(child, path);
        }
    }
    Ok(())
}

fn copy_into_zip(
    writer: &mut zip::ZipWriter<File>,
    options: zip::write::SimpleFileOptions,
    name: &str,
    source: &Path,
) -> Result<()> {
    let size = std::fs::metadata(source).at(source)?.len();
    writer.start_file(name, options.large_file(size >= u64::from(u32::MAX)))?;
    let mut input = File::open(source).at(source)?;
    std::io::copy(&mut input, writer).at(source)?;
    writer.flush().at(source)
}

/// Keeps the newest few update backups; they are as big as the mods folder.
fn prune(backups: &Path) -> Result<()> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(backups)
        .at(backups)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().ends_with(BACKUP_SUFFIX)))
        .collect();
    // The names start with a sortable timestamp.
    found.sort();
    let stale = found.len().saturating_sub(KEPT_BACKUPS);
    for path in found.into_iter().take(stale) {
        std::fs::remove_file(&path).at(&path)?;
    }
    Ok(())
}

/// Index entries (`mods/.index/*.pw.toml`) whose file is `file_name`, so removing a mod also removes its metadata.
pub(super) fn remove_mod_index(game_dir: &Path, file_name: &str) -> Result<()> {
    let index_dir = super::packwiz::index_dir(game_dir, super::ResourceKind::Mod);
    for entry in super::packwiz::list(&index_dir)? {
        if entry.data.filename == file_name {
            std::fs::remove_file(&entry.path).at(&entry.path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, id: &str) -> PackFileState {
        PackFileState { path: path.to_owned(), id: id.to_owned() }
    }

    #[test]
    fn diff_lists_only_what_the_old_pack_brought_and_the_new_one_dropped() {
        let old = PackState {
            files: vec![file("mods/a.jar", "1"), file("mods/b.jar", "2"), file("mods/c.jar", "3")],
            overrides: vec!["config/a.toml".into(), "config/old.toml".into()],
        };
        // b.jar is gone, c.jar keeps its path with new content (replaced, not removed) and
        // config/old.toml is no longer an override.
        let new_files = [file("mods/a.jar", "1"), file("mods/c.jar", "4"), file("mods/d.jar", "5")];
        let new_overrides = ["config/a.toml".to_owned()];
        let diff = old.diff(&new_files, &new_overrides);
        assert_eq!(diff.removed_files, ["mods/b.jar"]);
        assert_eq!(diff.removed_overrides, ["config/old.toml"]);
        assert!(old.has_file("mods/a.jar", "1"));
        assert!(!old.has_file("mods/c.jar", "4"));
    }

    #[test]
    fn state_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(PackState::load(dir.path()).unwrap(), PackState::default());
        let state = PackState { files: vec![file("mods/a.jar", "1")], overrides: vec!["config/x".into()] };
        state.save(dir.path()).unwrap();
        assert_eq!(PackState::load(dir.path()).unwrap(), state);
    }

    #[test]
    fn restore_brings_back_changed_deleted_and_removes_created_files() {
        let dir = tempfile::tempdir().unwrap();
        let (instance_dir, game_dir) = (dir.path().to_owned(), dir.path().join("minecraft"));
        std::fs::create_dir_all(game_dir.join("mods/.index")).unwrap();
        std::fs::create_dir_all(game_dir.join("config")).unwrap();
        std::fs::write(game_dir.join("mods/a.jar"), "a").unwrap();
        std::fs::write(game_dir.join("mods/.index/a.pw.toml"), "meta").unwrap();
        std::fs::write(game_dir.join("config/x.toml"), "old").unwrap();
        PackState { files: vec![file("mods/a.jar", "1")], overrides: vec![] }.save(&instance_dir).unwrap();

        let touched = ["config/x.toml".to_owned(), "config/new.toml".to_owned()];
        let backup = Backup::create(&instance_dir, &game_dir, &["mods"], &touched).unwrap();

        std::fs::remove_file(game_dir.join("mods/a.jar")).unwrap();
        std::fs::remove_file(game_dir.join("mods/.index/a.pw.toml")).unwrap();
        std::fs::write(game_dir.join("mods/b.jar"), "b").unwrap();
        std::fs::write(game_dir.join("config/x.toml"), "changed").unwrap();
        std::fs::write(game_dir.join("config/new.toml"), "new").unwrap();
        PackState::default().save(&instance_dir).unwrap();

        backup.restore().unwrap();
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/a.jar")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/.index/a.pw.toml")).unwrap(), "meta");
        assert!(!game_dir.join("mods/b.jar").exists());
        assert_eq!(std::fs::read_to_string(game_dir.join("config/x.toml")).unwrap(), "old");
        assert!(!game_dir.join("config/new.toml").exists());
        assert_eq!(PackState::load(&instance_dir).unwrap().files, [file("mods/a.jar", "1")]);
    }

    #[test]
    fn only_the_newest_backups_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(BACKUP_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        for stamp in ["20260101-000000", "20260102-000000", "20260103-000000", "20260104-000000"] {
            std::fs::write(backups.join(format!("{stamp}{BACKUP_SUFFIX}")), "").unwrap();
        }
        std::fs::write(backups.join("keep-me.txt"), "").unwrap();
        prune(&backups).unwrap();
        let mut left: Vec<String> =
            std::fs::read_dir(&backups).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(
            left,
            [
                "20260102-000000-modpack-update.zip",
                "20260103-000000-modpack-update.zip",
                "20260104-000000-modpack-update.zip",
                "keep-me.txt"
            ]
        );
    }
}
