//! Prism Launcher and MultiMC instances installed on this machine: finding them and reading one from
//! its folder (the zip variant is in `archive`). The originals are only ever read.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::archive::{ImportedInstance, PRISM_CONFIG, PRISM_PACK, parse_prism_config, prism_instance};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::content::Loader;
use crate::content::modcompat::{accepts, descriptors_of_jar, loader_of_components};
use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};

const DEFAULT_INSTANCE_DIR: &str = "instances";
const MOD_JAR_LIMIT: usize = 300;

/// Which launcher of the MultiMC family an instance folder belongs to, as far as can be told from its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PrismLauncher {
    Prism,
    MultiMC,
    PolyMC,
    UltimMC,
    Fjord,
}

impl PrismLauncher {
    pub(super) const ALL: [PrismLauncher; 5] = [Self::Prism, Self::MultiMC, Self::PolyMC, Self::UltimMC, Self::Fjord];

    /// The executable name without extension, lowercase.
    fn exe_stem(self) -> &'static str {
        match self {
            Self::Prism => "prismlauncher",
            Self::MultiMC => "multimc",
            Self::PolyMC => "polymc",
            Self::UltimMC => "ultimmc",
            Self::Fjord => "fjordlauncher",
        }
    }

    /// The launcher's executable by file name (case-insensitive, `.exe` optional).
    pub(super) fn from_exe_name(file_name: &str) -> Option<Self> {
        let lower = file_name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        Self::ALL.into_iter().find(|launcher| launcher.exe_stem() == stem)
    }

    fn settings_file(self) -> String {
        format!("{}.cfg", self.exe_stem())
    }

    /// The folder name of the launcher's standard data folder, `None` for launchers that are always portable.
    fn data_dir_name(self) -> Option<&'static str> {
        match self {
            Self::Prism => Some("PrismLauncher"),
            Self::PolyMC => Some("PolyMC"),
            Self::Fjord => Some("FjordLauncher"),
            Self::MultiMC | Self::UltimMC => None,
        }
    }
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
    /// The launcher's data folder (holds the instances folder and the settings), when known.
    pub launcher_dir: Option<PathBuf>,
    /// The loader the instance declares: `fabric`, `quilt`, `forge`, `neoforge`; `None` for vanilla.
    pub declared_loader: Option<Loader>,
    /// The one loader all loader-specific jars in `mods` target, when it differs from the declared one.
    /// `None` when they agree, there are none, or they are mixed.
    pub mod_loader: Option<Loader>,
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

/// The loader the jars in `<game dir>/mods` were made for, if all loader-specific ones agree on a loader
/// the instance's own loader (`declared`) does not accept. Reads only the descriptor entries of at most
/// `MOD_JAR_LIMIT` enabled jars; unreadable jars are ignored.
fn mismatching_mod_loader(mods_dir: &Path, declared: Option<Loader>) -> Option<Loader> {
    let mut common: Option<Vec<Loader>> = None;
    for entry in std::fs::read_dir(mods_dir).ok()?.filter_map(|entry| entry.ok()).filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))).take(MOD_JAR_LIMIT) {
        let Ok(file) = std::fs::File::open(entry.path()) else { continue };
        let Ok(descriptors) = descriptors_of_jar(file) else { continue };
        let mut targets: Vec<Loader> = descriptors.iter().map(|descriptor| descriptor.loader).collect();
        targets.dedup();
        if targets.is_empty() {
            continue;
        }
        // The declared loader runs this jar: nothing to ask about it, and it keeps the others from deciding.
        if declared.is_some_and(|loader| targets.iter().any(|target| accepts(loader, *target))) {
            return None;
        }
        common = Some(match common {
            None => targets,
            Some(previous) => previous.into_iter().filter(|loader| targets.contains(loader)).collect(),
        });
    }
    // A jar for both Fabric and Quilt (or Forge and NeoForge) leaves two candidates; the first of these wins.
    [Loader::Fabric, Loader::Quilt, Loader::NeoForge, Loader::Forge].into_iter().find(|loader| common.as_ref().is_some_and(|common| common.contains(loader)))
}

fn describe(dir: &Path, launcher: PrismLauncher, launcher_dir: Option<&Path>) -> Result<PrismInstanceInfo> {
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
    let declared_loader = loader_of_components(&imported.components);
    let mod_loader = mismatching_mod_loader(&dir.join(game_dir_name(dir)).join("mods"), declared_loader);
    Ok(PrismInstanceInfo {
        path: dir.to_owned(),
        name: imported.name,
        minecraft_version,
        loader,
        launcher,
        launcher_dir: launcher_dir.map(Path::to_owned),
        declared_loader,
        mod_loader,
    })
}

/// The components with the loader replaced by `loader` at `version` (added when the instance was vanilla).
pub fn with_loader(mut components: Vec<ComponentSpec>, loader: Loader, version: &str) -> Vec<ComponentSpec> {
    components.retain(|component| ![Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge].into_iter().any(|other| loader_uid(other) == component.uid));
    components.push(ComponentSpec { uid: loader_uid(loader).to_owned(), version: version.to_owned(), dependency_only: false, disabled: false });
    components
}

fn loader_uid(loader: Loader) -> &'static str {
    match loader {
        Loader::Fabric => "net.fabricmc.fabric-loader",
        Loader::Quilt => "org.quiltmc.quilt-loader",
        Loader::Forge => "net.minecraftforge",
        Loader::NeoForge => "net.neoforged",
    }
}

/// The instances folder a data folder uses: `InstanceDir` of its settings file (absolute, or relative to
/// the data folder), `instances` by default. The launcher is told by which settings file exists.
fn instances_dir_of(data_dir: &Path) -> Option<(PathBuf, PrismLauncher)> {
    let (settings_file, launcher) = PrismLauncher::ALL.into_iter().map(|launcher| (data_dir.join(launcher.settings_file()), launcher)).find(|(file, _)| file.is_file())?;
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

/// The instances of a launcher data folder. `launcher` is used when the folder has no settings file yet
/// (a launcher writes it on exit; a fresh install may only have the folder).
pub(super) fn scan_data_dir(data_dir: &Path, launcher: PrismLauncher, out: &mut Vec<PrismInstanceInfo>) {
    let (instances, launcher) = instances_dir_of(data_dir).unwrap_or_else(|| (data_dir.join(DEFAULT_INSTANCE_DIR), launcher));
    scan_instances_dir(&instances, launcher, Some(data_dir), out);
}

/// Folders Prism and MultiMC use for their own bookkeeping (`_MMC_TEMP`, `.LAUNCHER_TEMP`, ...).
fn is_bookkeeping(name: &str) -> bool {
    name.starts_with('_') || name.starts_with('.')
}

fn scan_instances_dir(dir: &Path, launcher: PrismLauncher, launcher_dir: Option<&Path>, out: &mut Vec<PrismInstanceInfo>) {
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
        match describe(&path, launcher, launcher_dir) {
            Ok(info) => out.push(info),
            Err(error) => tracing::warn!("skipping the instance {}: {error}", path.display()),
        }
    }
}

/// Standard data folders of the launchers that have one, and (on Linux) Flatpak's Prism.
pub(super) fn standard_data_dirs() -> Vec<(PathBuf, PrismLauncher)> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        for launcher in PrismLauncher::ALL {
            if let Some(name) = launcher.data_dir_name() {
                dirs.push((data.join(name), launcher));
            }
        }
    }
    if cfg!(target_os = "linux")
        && let Some(home) = dirs::home_dir()
    {
        dirs.push((home.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher"), PrismLauncher::Prism));
    }
    dirs
}

/// The data folder of a launcher found by its executable in `exe_dir`: portable if it says so (`portable.txt`,
/// settings or instances beside the executable) or the launcher is always portable (MultiMC, UltimMC).
pub(super) fn portable_data_dir(exe_dir: &Path, launcher: PrismLauncher) -> Option<PathBuf> {
    let portable = launcher.data_dir_name().is_none()
        || exe_dir.join("portable.txt").is_file()
        || exe_dir.join(launcher.settings_file()).is_file()
        || exe_dir.join(DEFAULT_INSTANCE_DIR).is_dir();
    portable.then(|| exe_dir.to_owned())
}

/// Instances of the launchers in the standard places, or, with `folder`, of that folder: a
/// single instance, an instances folder, or a launcher's data folder (MultiMC is portable, so it has no
/// standard place; see `discover_launcher_instances`). Broken instances are logged and left out.
pub fn find_prism_instances(folder: Option<&Path>) -> Vec<PrismInstanceInfo> {
    let mut found = Vec::new();
    match folder {
        Some(folder) => scan_folder(folder, &mut found),
        None => {
            for (data_dir, launcher) in standard_data_dirs() {
                scan_data_dir(&data_dir, launcher, &mut found);
            }
        }
    }
    found.sort_by_key(|info| info.name.to_lowercase());
    found
}

fn scan_folder(folder: &Path, out: &mut Vec<PrismInstanceInfo>) {
    // The launcher is told by the settings file of the data folder, which holds the instances folder.
    let owner = |data_dir: Option<&Path>| data_dir.and_then(|dir| instances_dir_of(dir).map(|(_, launcher)| (dir.to_owned(), launcher)));
    if folder.join(PRISM_CONFIG).is_file() {
        let owner = owner(folder.parent().and_then(Path::parent));
        let launcher = owner.as_ref().map_or(PrismLauncher::MultiMC, |(_, launcher)| *launcher);
        match describe(folder, launcher, owner.as_ref().map(|(dir, _)| dir.as_path())) {
            Ok(info) => out.push(info),
            Err(error) => tracing::warn!("skipping the instance {}: {error}", folder.display()),
        }
    } else if instances_dir_of(folder).is_some() {
        scan_data_dir(folder, PrismLauncher::MultiMC, out);
    } else {
        let owner = owner(folder.parent());
        let launcher = owner.as_ref().map_or(PrismLauncher::MultiMC, |(_, launcher)| *launcher);
        scan_instances_dir(folder, launcher, owner.as_ref().map(|(dir, _)| dir.as_path()), out);
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

        std::fs::write(data.join("prismlauncher.cfg"), "[General]\nLanguage=en\n").unwrap();
        assert_eq!(instances_dir_of(&data), Some((data.join("instances"), PrismLauncher::Prism)));
        std::fs::write(data.join("prismlauncher.cfg"), "InstanceDir=games\n").unwrap();
        assert_eq!(instances_dir_of(&data), Some((data.join("games"), PrismLauncher::Prism)));
        let absolute = dir.path().join("elsewhere");
        std::fs::write(data.join("prismlauncher.cfg"), format!("InstanceDir={}\n", absolute.display().to_string().replace('\\', "\\\\"))).unwrap();
        assert_eq!(instances_dir_of(&data), Some((absolute, PrismLauncher::Prism)));

        let portable = dir.path().join("multimc");
        std::fs::create_dir(&portable).unwrap();
        std::fs::write(portable.join("multimc.cfg"), "").unwrap();
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
        std::fs::write(dir.path().join("prismlauncher.cfg"), "InstanceDir=instances\n").unwrap();
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

    fn write_jar(path: &Path, entries: &[&str]) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for name in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            let body = if name.ends_with(".toml") { "modLoader=\"javafml\"\nloaderVersion=\"[1,)\"\n[[mods]]\nmodId=\"x\"\nversion=\"1\"\n" } else { r#"{"id":"x","version":"1","schemaVersion":1,"quilt_loader":{"id":"x","version":"1"}}"# };
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn detects_the_loader_the_mods_were_made_for() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        assert_eq!(mismatching_mod_loader(&mods, Some(Loader::Fabric)), None);

        write_jar(&mods.join("a.jar"), &["fabric.mod.json"]);
        write_jar(&mods.join("b.jar"), &["fabric.mod.json", "quilt.mod.json"]);
        write_jar(&mods.join("off.jar.disabled"), &["META-INF/neoforge.mods.toml"]);
        write_jar(&mods.join("lib.jar"), &["readme.txt"]);
        std::fs::write(mods.join("broken.jar"), "not a zip").unwrap();
        assert_eq!(mismatching_mod_loader(&mods, Some(Loader::NeoForge)), Some(Loader::Fabric));
        assert_eq!(mismatching_mod_loader(&mods, None), Some(Loader::Fabric));
        assert_eq!(mismatching_mod_loader(&mods, Some(Loader::Fabric)), None);
        // Quilt runs Fabric mods
        assert_eq!(mismatching_mod_loader(&mods, Some(Loader::Quilt)), None);

        // mixed loaders: nothing to suggest
        write_jar(&mods.join("c.jar"), &["META-INF/mods.toml"]);
        assert_eq!(mismatching_mod_loader(&mods, Some(Loader::Quilt)), None);
    }

    #[test]
    fn scanning_reports_declared_and_mod_loader() {
        let dir = tempfile::tempdir().unwrap();
        let instance = write_instance(dir.path(), "a", Some(".minecraft"));
        write_jar(&instance.join(".minecraft/mods/m.jar"), &["META-INF/mods.toml"]);
        let info = &find_prism_instances(Some(&instance))[0];
        assert_eq!(info.declared_loader, Some(Loader::Fabric));
        assert_eq!(info.mod_loader, Some(Loader::Forge));
    }

    #[test]
    fn replaces_the_loader_component() {
        let spec = |uid: &str, version: &str| ComponentSpec { uid: uid.into(), version: version.into(), dependency_only: false, disabled: false };
        let components = vec![spec(MINECRAFT_UID, "1.21.11"), spec("net.neoforged", "21.11.48")];
        let replaced = with_loader(components.clone(), Loader::Fabric, "0.18.1");
        assert_eq!(replaced, [spec(MINECRAFT_UID, "1.21.11"), spec("net.fabricmc.fabric-loader", "0.18.1")]);
        assert_eq!(with_loader(vec![spec(MINECRAFT_UID, "1.21.11")], Loader::Quilt, "1").len(), 2);
    }
}
