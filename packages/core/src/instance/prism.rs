//! Prism Launcher and MultiMC instances installed on this machine: finding them and reading one from
//! its folder (the zip variant is in `archive`). The originals are only ever read.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::archive::{ImportedInstance, PRISM_CONFIG, PRISM_PACK, parse_prism_config, prism_instance};
use crate::component::MINECRAFT_UID;
use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};

const PRISM_SETTINGS: &str = "prismlauncher.cfg";
const MULTIMC_SETTINGS: &str = "multimc.cfg";
const DEFAULT_INSTANCE_DIR: &str = "instances";
const PRISM_DATA_DIR: &str = "PrismLauncher";

/// Which launcher an instance folder belongs to, as far as can be told from its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PrismLauncher {
    Prism,
    MultiMC,
}

/// An instance found on disk, ready to be offered for import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrismInstanceInfo {
    /// The instance folder (holds `instance.cfg`).
    pub path: PathBuf,
    pub name: String,
    pub minecraft_version: String,
    /// The mod loader with its version, e.g. `Fabric 0.15.11`; `None` for vanilla.
    pub loader: Option<String>,
    pub launcher: PrismLauncher,
}

/// The folder that holds the game files of a Prism instance: `.minecraft`, or `minecraft` for old MultiMC.
fn game_dir_name(instance_dir: &Path) -> &'static str {
    if instance_dir.join(".minecraft").is_dir() || !instance_dir.join("minecraft").is_dir() { ".minecraft" } else { "minecraft" }
}

/// Reads an installed instance folder (`instance.cfg` and `mmc-pack.json`). `source.game_prefix` is the
/// game dir relative to `dir`, with a trailing `/`.
pub fn read_prism_dir(dir: &Path) -> Result<ImportedInstance> {
    let config_path = dir.join(PRISM_CONFIG);
    let config = std::fs::read(&config_path).at(&config_path)?;
    let pack_path = dir.join(PRISM_PACK);
    let pack = std::fs::read(&pack_path).at(&pack_path)?;
    prism_instance(&config, &pack, format!("{}/", game_dir_name(dir)))
}

fn loader_name(uid: &str) -> Option<&'static str> {
    match uid {
        "net.fabricmc.fabric-loader" => Some("Fabric"),
        "org.quiltmc.quilt-loader" => Some("Quilt"),
        "net.minecraftforge" => Some("Forge"),
        "net.neoforged" => Some("NeoForge"),
        _ => None,
    }
}

fn describe(dir: &Path, launcher: PrismLauncher) -> Result<PrismInstanceInfo> {
    let imported = read_prism_dir(dir)?;
    let minecraft_version = imported
        .components
        .iter()
        .find(|component| component.uid == MINECRAFT_UID)
        .map(|component| component.version.clone())
        .ok_or_else(|| Error::Other("the instance does not declare a Minecraft version".into()))?;
    let loader = imported
        .components
        .iter()
        .find_map(|component| loader_name(&component.uid).map(|name| format!("{name} {}", component.version)));
    Ok(PrismInstanceInfo { path: dir.to_owned(), name: imported.name, minecraft_version, loader, launcher })
}

/// The instances folder a data folder uses: `InstanceDir` of its settings file (absolute, or relative to
/// the data folder), `instances` by default. The launcher is told by which settings file exists.
fn instances_dir_of(data_dir: &Path) -> Option<(PathBuf, PrismLauncher)> {
    let (settings_file, launcher) = if data_dir.join(PRISM_SETTINGS).is_file() {
        (data_dir.join(PRISM_SETTINGS), PrismLauncher::Prism)
    } else if data_dir.join(MULTIMC_SETTINGS).is_file() {
        (data_dir.join(MULTIMC_SETTINGS), PrismLauncher::MultiMC)
    } else {
        return None;
    };
    let configured = match std::fs::read_to_string(&settings_file) {
        Ok(text) => parse_prism_config(&text).remove("InstanceDir").filter(|dir| !dir.is_empty()),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", settings_file.display());
            None
        }
    };
    // `join` with an absolute path replaces the base, which is what an absolute `InstanceDir` means.
    Some((data_dir.join(configured.as_deref().unwrap_or(DEFAULT_INSTANCE_DIR)), launcher))
}

/// Folders Prism and MultiMC use for their own bookkeeping (`_MMC_TEMP`, `.LAUNCHER_TEMP`, ...).
fn is_bookkeeping(name: &str) -> bool {
    name.starts_with('_') || name.starts_with('.')
}

fn scan_instances_dir(dir: &Path, launcher: PrismLauncher, out: &mut Vec<PrismInstanceInfo>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!("cannot read the instances folder {}: {error}", dir.display());
            return;
        }
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) if !is_bookkeeping(&entry.file_name().to_string_lossy()) => entry.path(),
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!("cannot list {}: {error}", dir.display());
                continue;
            }
        };
        if !path.join(PRISM_CONFIG).is_file() {
            continue;
        }
        match describe(&path, launcher) {
            Ok(info) => out.push(info),
            Err(error) => tracing::warn!("skipping the instance {}: {error}", path.display()),
        }
    }
}

/// Data folders of Prism Launcher on this machine, native and (on Linux) Flatpak.
fn prism_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join(PRISM_DATA_DIR));
    }
    if cfg!(target_os = "linux")
        && let Some(home) = dirs::home_dir()
    {
        dirs.push(home.join(".var/app/org.prismlauncher.PrismLauncher/data").join(PRISM_DATA_DIR));
    }
    dirs
}

/// Instances of the Prism installations in the standard places, or, with `folder`, of that folder: a
/// single instance, an instances folder, or a launcher's data folder (MultiMC is portable, so it has no
/// standard place). Broken instances are logged and left out.
pub fn find_prism_instances(folder: Option<&Path>) -> Vec<PrismInstanceInfo> {
    let mut found = Vec::new();
    match folder {
        Some(folder) => scan_folder(folder, &mut found),
        None => {
            for data_dir in prism_data_dirs() {
                if let Some((instances, launcher)) = instances_dir_of(&data_dir) {
                    scan_instances_dir(&instances, launcher, &mut found);
                } else {
                    // Prism writes its settings file on exit; a fresh install may only have the folder.
                    scan_instances_dir(&data_dir.join(DEFAULT_INSTANCE_DIR), PrismLauncher::Prism, &mut found);
                }
            }
        }
    }
    found.sort_by_key(|info| info.name.to_lowercase());
    found
}

fn scan_folder(folder: &Path, out: &mut Vec<PrismInstanceInfo>) {
    // MultiMC keeps its settings next to the instances folder, Prism's data folder holds both.
    let launcher = match folder.parent() {
        Some(parent) if parent.join(PRISM_SETTINGS).is_file() => PrismLauncher::Prism,
        _ if folder.join(PRISM_SETTINGS).is_file() => PrismLauncher::Prism,
        _ => PrismLauncher::MultiMC,
    };
    if folder.join(PRISM_CONFIG).is_file() {
        match describe(folder, launcher) {
            Ok(info) => out.push(info),
            Err(error) => tracing::warn!("skipping the instance {}: {error}", folder.display()),
        }
    } else if let Some((instances, launcher)) = instances_dir_of(folder) {
        scan_instances_dir(&instances, launcher, out);
    } else {
        scan_instances_dir(folder, launcher, out);
    }
}

/// Copies the game dir of the instance `source` (as `read_prism_dir` located it) into `game_dir`. Links
/// are skipped: they may point anywhere, and Prism uses none for its own content. Progress counts files.
pub fn copy_prism_game_dir(instance_dir: &Path, game_prefix: &str, game_dir: &Path, task: &str, progress: &ProgressFn) -> Result<()> {
    let source = instance_dir.join(game_prefix.trim_end_matches('/'));
    if !source.is_dir() {
        return Ok(());
    }
    let stage = crate::i18n::tr("Copying files", "Копирование файлов");
    let mut files = Vec::new();
    list_files(&source, Path::new(""), &mut files)?;
    let total = files.len() as u64;
    progress(ProgressEvent { task: task.to_owned(), stage: stage.to_owned(), done: 0, total });
    std::fs::create_dir_all(game_dir).at(game_dir)?;
    for (done, relative) in files.iter().enumerate() {
        let (from, to) = (source.join(relative), game_dir.join(relative));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        std::fs::copy(&from, &to).at(&from)?;
        progress(ProgressEvent { task: task.to_owned(), stage: stage.to_owned(), done: done as u64 + 1, total });
    }
    Ok(())
}

/// Regular files below `dir`, relative to the start of the walk; links are left out.
fn list_files(dir: &Path, relative: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).at(dir)? {
        let entry = entry.at(dir)?;
        let kind = entry.file_type().at(entry.path())?;
        let entry_relative = relative.join(entry.file_name());
        if kind.is_dir() {
            list_files(&entry.path(), &entry_relative, out)?;
        } else if kind.is_file() {
            out.push(entry_relative);
        } else {
            // Links are not followed (no loops, nothing read from outside the instance), so linked worlds or mod folders stay behind.
            tracing::warn!("not importing {}: links and special files are skipped", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK: &str = r#"{"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.15.11"}]}"#;

    fn write_instance(dir: &Path, name: &str, game_dir: Option<&str>) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(PRISM_CONFIG), format!("[General]\nname={name} pack\nnotes=hi\n")).unwrap();
        std::fs::write(path.join(PRISM_PACK), PACK).unwrap();
        if let Some(game_dir) = game_dir {
            std::fs::create_dir_all(path.join(game_dir).join("config")).unwrap();
            std::fs::write(path.join(game_dir).join("config/a.toml"), "x").unwrap();
        }
        path
    }

    #[test]
    fn reads_an_instance_folder() {
        let dir = tempfile::tempdir().unwrap();
        let modern = read_prism_dir(&write_instance(dir.path(), "a", Some(".minecraft"))).unwrap();
        assert_eq!(modern.name, "a pack");
        assert_eq!(modern.source.notes, "hi");
        assert_eq!(modern.source.game_prefix, ".minecraft/");
        assert_eq!(read_prism_dir(&write_instance(dir.path(), "b", Some("minecraft"))).unwrap().source.game_prefix, "minecraft/");
        let missing = dir.path().join("none");
        std::fs::create_dir(&missing).unwrap();
        assert!(read_prism_dir(&missing).is_err());
    }

    #[test]
    fn instance_dir_setting_is_relative_absolute_or_default() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        assert_eq!(instances_dir_of(&data), None);

        std::fs::write(data.join(PRISM_SETTINGS), "[General]\nLanguage=en\n").unwrap();
        assert_eq!(instances_dir_of(&data), Some((data.join("instances"), PrismLauncher::Prism)));
        std::fs::write(data.join(PRISM_SETTINGS), "InstanceDir=games\n").unwrap();
        assert_eq!(instances_dir_of(&data), Some((data.join("games"), PrismLauncher::Prism)));
        let absolute = dir.path().join("elsewhere");
        std::fs::write(data.join(PRISM_SETTINGS), format!("InstanceDir={}\n", absolute.display().to_string().replace('\\', "\\\\"))).unwrap();
        assert_eq!(instances_dir_of(&data), Some((absolute, PrismLauncher::Prism)));

        let portable = dir.path().join("multimc");
        std::fs::create_dir(&portable).unwrap();
        std::fs::write(portable.join(MULTIMC_SETTINGS), "").unwrap();
        assert_eq!(instances_dir_of(&portable), Some((portable.join("instances"), PrismLauncher::MultiMC)));
    }

    #[test]
    fn scans_instances_and_skips_junk_and_broken_ones() {
        let dir = tempfile::tempdir().unwrap();
        let instances = dir.path().join("instances");
        write_instance(&instances, "zeta", Some(".minecraft"));
        write_instance(&instances, "Alpha", None);
        write_instance(&instances, "_MMC_TEMP", None);
        write_instance(&instances, ".tmp", None);
        std::fs::create_dir_all(instances.join("no-config")).unwrap();
        std::fs::write(instances.join("instgroups.json"), "{}").unwrap();
        let broken = write_instance(&instances, "broken", None);
        std::fs::write(broken.join(PRISM_PACK), "not json").unwrap();

        let found = find_prism_instances(Some(&instances));
        let names: Vec<_> = found.iter().map(|info| info.name.as_str()).collect();
        assert_eq!(names, ["Alpha pack", "zeta pack"]);
        assert_eq!(found[0].minecraft_version, "1.20.1");
        assert_eq!(found[0].loader.as_deref(), Some("Fabric 0.15.11"));
        assert_eq!(found[0].launcher, PrismLauncher::MultiMC);

        // A single instance folder and a data folder with a settings file work too.
        assert_eq!(find_prism_instances(Some(&instances.join("zeta"))).len(), 1);
        std::fs::write(dir.path().join(PRISM_SETTINGS), "InstanceDir=instances\n").unwrap();
        let from_data = find_prism_instances(Some(dir.path()));
        assert_eq!(from_data.len(), 2);
        assert!(from_data.iter().all(|info| info.launcher == PrismLauncher::Prism));
        assert!(find_prism_instances(Some(&dir.path().join("missing"))).is_empty());
    }

    #[test]
    fn copies_the_game_dir_without_touching_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let instance = write_instance(dir.path(), "a", Some(".minecraft"));
        std::fs::write(instance.join(".minecraft/options.txt"), "fov:0").unwrap();
        let game_dir = dir.path().join("out/game");
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let progress: ProgressFn = std::sync::Arc::new(move |event| sink.lock().unwrap().push((event.done, event.total)));
        copy_prism_game_dir(&instance, ".minecraft/", &game_dir, "t", &progress).unwrap();
        assert_eq!(std::fs::read_to_string(game_dir.join("config/a.toml")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(game_dir.join("options.txt")).unwrap(), "fov:0");
        assert!(instance.join(".minecraft/options.txt").is_file());
        assert_eq!(events.lock().unwrap().last(), Some(&(2, 2)));

        let empty = dir.path().join("empty");
        copy_prism_game_dir(&instance, "minecraft/", &empty, "t", &progress).unwrap();
        assert!(!empty.exists());
    }
}
