//! Instance zips: exporting an instance and reading back our own zips and Prism/MultiMC ones.
//!
//! Our layout: `instance.json`, optionally `pack-state.json`, and the game dir as `minecraft/...`.
//! Prism's: `instance.cfg`, `mmc-pack.json` and `.minecraft/` (or `minecraft/`), maybe under one top folder.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::fsutil::is_link;
use super::icon::{MAX_ICON_BYTES, custom_file_name};
use super::{INSTANCE_FILE, Instance, InstanceSettings, ManagedPack};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::content::PackState;
use crate::content::pack_state::PACK_STATE_FILE;
use crate::content::safe_join;
use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};
use crate::settings::MemorySettings;

const GAME_DIR_IN_ZIP: &str = "minecraft";
const PRISM_CONFIG: &str = "instance.cfg";
const PRISM_PACK: &str = "mmc-pack.json";
const TASK: &str = "export";

/// Prism's export dialog leaves these out by default (`ExportInstanceDialog.cpp`).
const DEFAULT_EXCLUDED: [&str; 5] = ["logs", "crash-reports", ".cache", ".fabric", ".quilt"];
const IGNORED_NAMES: [&str; 3] = [".DS_Store", "thumbs.db", "Thumbs.db"];

/// Loader uids of the Prism meta; everything else in `mmc-pack.json` is derived or unsupported.
const KNOWN_COMPONENTS: [&str; 5] = [
    MINECRAFT_UID,
    "net.fabricmc.fabric-loader",
    "org.quiltmc.quilt-loader",
    "net.minecraftforge",
    "net.neoforged",
];

/// The game dir paths an export leaves out: the given paths with everything below them.
#[derive(Debug, Clone, Default)]
pub struct ExportFilter {
    excluded: Vec<String>,
}

impl ExportFilter {
    pub fn new(excluded: impl IntoIterator<Item = String>) -> Self {
        Self { excluded: excluded.into_iter().map(|path| path.trim_matches('/').to_lowercase()).collect() }
    }

    /// True when `path` (relative to the game dir, `/` separators) is exported.
    pub fn allows(&self, path: &str) -> bool {
        let lower = path.to_lowercase();
        let name = path.rsplit('/').next().unwrap_or(path);
        !IGNORED_NAMES.contains(&name)
            && !self.excluded.iter().any(|excluded| lower == *excluded || lower.starts_with(&format!("{excluded}/")))
    }
}

/// One row of the export file picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEntry {
    pub name: String,
    /// Relative to the game dir, `/` separators.
    pub path: String,
    pub is_dir: bool,
    /// Bytes; `None` for folders (summing them would walk the whole tree).
    pub size: Option<u64>,
    /// A shared folder (junction or symlink): exported as a copy of its content, or skipped.
    pub linked: bool,
    pub default_excluded: bool,
}

/// The entries of `dir` (relative to the game dir; empty for the root), folders first.
pub fn list_export_dir(game_dir: &Path, dir: &str) -> Result<Vec<ExportEntry>> {
    let absolute = if dir.is_empty() { game_dir.to_owned() } else { safe_join(game_dir, dir)? };
    let entries = match std::fs::read_dir(&absolute) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(&absolute),
    };
    let mut listed = Vec::new();
    for entry in entries {
        let entry = entry.at(&absolute)?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if dir.is_empty() { name.clone() } else { format!("{}/{name}", dir.trim_matches('/')) };
        let metadata = std::fs::metadata(&path).at(&path)?;
        listed.push(ExportEntry {
            default_excluded: !ExportFilter::new(DEFAULT_EXCLUDED.map(str::to_owned)).allows(&relative),
            linked: is_link(&path),
            is_dir: metadata.is_dir(),
            size: metadata.is_file().then_some(metadata.len()),
            name,
            path: relative,
        });
    }
    listed.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(listed)
}

/// The paths the picker starts with unchecked.
pub fn default_excluded() -> Vec<String> {
    DEFAULT_EXCLUDED.map(str::to_owned).to_vec()
}

/// A game dir file selected for export.
#[derive(Debug, Clone)]
pub struct ExportFile {
    /// Relative to the game dir, `/` separators.
    pub path: String,
    pub absolute: PathBuf,
}

/// Every file of the game dir the filter allows. Links are followed, so a shared folder's files
/// are listed as the instance's own; they never become links in an archive.
pub fn collect_files(game_dir: &Path, filter: &ExportFilter) -> Result<Vec<ExportFile>> {
    let mut files = Vec::new();
    collect_dir(game_dir, "", filter, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn collect_dir(dir: &Path, relative: &str, filter: &ExportFilter, out: &mut Vec<ExportFile>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && relative.is_empty() => return Ok(()),
        Err(error) => return Err(error).at(dir),
    };
    for entry in entries {
        let entry = entry.at(dir)?;
        let path = entry.path();
        let child = if relative.is_empty() {
            entry.file_name().to_string_lossy().into_owned()
        } else {
            format!("{relative}/{}", entry.file_name().to_string_lossy())
        };
        if !filter.allows(&child) {
            continue;
        }
        if std::fs::metadata(&path).at(&path)?.is_dir() {
            collect_dir(&path, &child, filter, out)?;
        } else {
            out.push(ExportFile { path: child, absolute: path });
        }
    }
    Ok(())
}

/// Entries that are already compressed are stored instead of deflated again.
pub fn zip_options(name: &str, size: u64) -> zip::write::SimpleFileOptions {
    let compressed = ["jar", "zip", "png", "ogg", "mrpack", "litemod"]
        .iter()
        .any(|extension| name.rsplit_once('.').is_some_and(|(_, found)| found.eq_ignore_ascii_case(extension)));
    let method = if compressed { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
    zip::write::SimpleFileOptions::default().compression_method(method).large_file(size >= u64::from(u32::MAX))
}

/// Adds a file from disk to the archive under `name`.
pub fn add_file<W: std::io::Write + Seek>(writer: &mut zip::ZipWriter<W>, name: &str, source: &Path) -> Result<()> {
    let size = std::fs::metadata(source).at(source)?.len();
    writer.start_file(name, zip_options(name, size))?;
    let mut input = File::open(source).at(source)?;
    std::io::copy(&mut input, writer).at(source)?;
    Ok(())
}

/// Writes the instance as a zip: `instance.json`, `pack-state.json` if it has one, and the allowed
/// game dir files under `minecraft/`. The archive is written beside the destination and renamed.
pub fn export_instance(
    instance: &Instance,
    instance_dir: &Path,
    game_dir: &Path,
    destination: &Path,
    filter: &ExportFilter,
    progress: &ProgressFn,
) -> Result<()> {
    let files = collect_files(game_dir, filter)?;
    let total = files.len() as u64;
    let report = |done| progress(ProgressEvent { task: TASK.to_owned(), stage: "Packing files".to_owned(), done, total });
    report(0);

    let temporary = destination.with_extension("zip.part");
    let result = (|| -> Result<()> {
        let mut writer = zip::ZipWriter::new(File::create(&temporary).at(&temporary)?);
        writer.start_file(INSTANCE_FILE, zip_options(INSTANCE_FILE, 0))?;
        serde_json::to_writer_pretty(&mut writer, instance)?;
        let state = instance_dir.join(PACK_STATE_FILE);
        if state.is_file() {
            add_file(&mut writer, PACK_STATE_FILE, &state)?;
        }
        if let Some(icon) = custom_file_name(&instance.icon) {
            let icon_path = instance_dir.join(icon);
            if icon_path.is_file() {
                add_file(&mut writer, icon, &icon_path)?;
            }
        }
        for (index, file) in files.iter().enumerate() {
            add_file(&mut writer, &format!("{GAME_DIR_IN_ZIP}/{}", file.path), &file.absolute)?;
            report(index as u64 + 1);
        }
        writer.finish()?;
        std::fs::rename(&temporary, destination).at(destination)
    })();
    if result.is_err() {
        // Best effort: the export error is the one worth reporting.
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Everything an instance zip carries besides the game dir files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSource {
    /// Archive folder holding the game dir files, with a trailing `/`.
    pub game_prefix: String,
    pub notes: String,
    pub icon: Option<String>,
    pub settings: InstanceSettings,
    pub managed_pack: Option<ManagedPack>,
    pub pack_state: Option<PackState>,
}

/// An instance zip, understood.
pub struct ImportedInstance {
    pub name: String,
    pub components: Vec<ComponentSpec>,
    pub source: InstanceSource,
}

fn read_entry(archive: &mut zip::ZipArchive<impl Read + Seek>, name: &str) -> Result<Option<Vec<u8>>> {
    if archive.index_for_name(name).is_none() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    archive.by_name(name)?.read_to_end(&mut bytes).at(name)?;
    Ok(Some(bytes))
}

/// Our own zip: `instance.json` at the root. `None` when the archive is not one.
pub fn read_k0t1k_zip(archive: &mut zip::ZipArchive<impl Read + Seek>) -> Result<Option<ImportedInstance>> {
    let Some(bytes) = read_entry(archive, INSTANCE_FILE)? else {
        return Ok(None);
    };
    let instance = Instance::from_json(&bytes)?;
    let pack_state = read_entry(archive, PACK_STATE_FILE)?.map(|bytes| serde_json::from_slice(&bytes)).transpose()?;
    let mut settings = instance.settings;
    // A Java path belongs to the machine the instance was exported from.
    if let Some(java) = &mut settings.java {
        java.path = None;
    }
    Ok(Some(ImportedInstance {
        name: instance.name,
        components: instance.components.into_iter().filter(|component| !component.dependency_only).collect(),
        source: InstanceSource {
            game_prefix: format!("{GAME_DIR_IN_ZIP}/"),
            notes: instance.notes,
            icon: Some(instance.icon),
            settings,
            managed_pack: instance.managed_pack,
            pack_state,
        },
    }))
}

/// The custom icon image of one of our zips, which keeps it at the root under the name `instance.json` gives.
/// `None` when `icon` is not a custom icon or the archive lacks the file.
pub fn read_custom_icon(zip_path: &Path, icon: &str) -> Result<Option<Vec<u8>>> {
    let Some(name) = custom_file_name(icon) else {
        return Ok(None);
    };
    let mut archive = zip::ZipArchive::new(File::open(zip_path).at(zip_path)?)?;
    let Some(entry) = archive.index_for_name(name) else {
        return Ok(None);
    };
    let entry = archive.by_index(entry)?;
    if entry.size() > MAX_ICON_BYTES {
        return Err(Error::Other(format!("the icon {name} in the archive is too big")));
    }
    let mut bytes = Vec::new();
    entry.take(MAX_ICON_BYTES).read_to_end(&mut bytes).at(name)?;
    Ok(Some(bytes))
}

/// A Prism or MultiMC zip: the shallowest `instance.cfg` marks the instance root. `None` when absent.
pub fn read_prism_zip(archive: &mut zip::ZipArchive<impl Read + Seek>) -> Result<Option<ImportedInstance>> {
    let mut config_path: Option<String> = None;
    for index in 0..archive.len() {
        let name = archive.by_index(index)?.name()?.into_owned();
        let is_config = name == PRISM_CONFIG || name.ends_with(&format!("/{PRISM_CONFIG}"));
        if is_config && config_path.as_ref().is_none_or(|best| name.len() < best.len()) {
            config_path = Some(name);
        }
    }
    let Some(config_path) = config_path else {
        return Ok(None);
    };
    let root = config_path.strip_suffix(PRISM_CONFIG).unwrap_or_default().to_owned();
    let config = read_entry(archive, &config_path)?.unwrap_or_default();
    let pack = read_entry(archive, &format!("{root}{PRISM_PACK}"))?
        .ok_or_else(|| Error::Other(format!("the Prism instance has no {PRISM_PACK}")))?;

    let config = parse_prism_config(&String::from_utf8_lossy(&config));
    let components = map_prism_components(&pack)?;
    // Prism names the game dir `.minecraft`; old MultiMC versions used `minecraft`.
    let dot_minecraft = format!("{root}.minecraft/");
    let mut has_dot_minecraft = false;
    for index in 0..archive.len() {
        has_dot_minecraft |= archive.by_index(index)?.name()?.starts_with(&dot_minecraft);
    }
    let game_prefix = if has_dot_minecraft { dot_minecraft } else { format!("{root}minecraft/") };

    let name = config.get("name").filter(|name| !name.is_empty()).cloned().unwrap_or_else(|| "Prism instance".to_owned());
    let flag = |key: &str| config.get(key).is_some_and(|value| value == "true");
    let number = |key: &str| config.get(key).and_then(|value| value.parse::<u32>().ok());
    let mut settings = InstanceSettings::default();
    if flag("OverrideMemory")
        && let Some(max_mib) = number("MaxMemAlloc")
    {
        let min_mib = number("MinMemAlloc").unwrap_or(MemorySettings::default().min_mib).min(max_mib);
        settings.memory = Some(MemorySettings { min_mib, max_mib, auto: false });
    }
    if flag("OverrideJavaArgs") {
        settings.jvm_args = config.get("JvmArgs").cloned();
    }
    // Playtime restarts at zero and the icon key is Prism's own, so neither is carried over.
    Ok(Some(ImportedInstance {
        name,
        components,
        source: InstanceSource {
            game_prefix,
            notes: config.get("notes").cloned().unwrap_or_default(),
            icon: None,
            settings,
            managed_pack: None,
            pack_state: None,
        },
    }))
}

/// `key=value` lines of Qt's INI, escapes undone and surrounding quotes dropped. Section headers are skipped.
fn parse_prism_config(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let value = value.trim();
            let value = value.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')).unwrap_or(value);
            (key.trim().to_owned(), unescape(value))
        })
        .collect()
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// `mmc-pack.json` components, reduced to Minecraft and the loader. Prism's uids are the same meta
/// uids this launcher uses; LWJGL, intermediary and the like are dependency-only and re-derived.
fn map_prism_components(pack: &[u8]) -> Result<Vec<ComponentSpec>> {
    let value: serde_json::Value = serde_json::from_slice(pack)?;
    let mut components = Vec::new();
    for entry in value.get("components").and_then(|components| components.as_array()).into_iter().flatten() {
        let (Some(uid), Some(version)) =
            (entry.get("uid").and_then(|uid| uid.as_str()), entry.get("version").and_then(|version| version.as_str()))
        else {
            continue;
        };
        if KNOWN_COMPONENTS.contains(&uid) {
            components.push(ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false });
        } else if !entry.get("dependencyOnly").and_then(|flag| flag.as_bool()).unwrap_or(false) {
            tracing::warn!("ignoring unsupported Prism component {uid}");
        }
    }
    if !components.iter().any(|component| component.uid == MINECRAFT_UID) {
        return Err(Error::Other("the Prism instance does not declare a Minecraft version".into()));
    }
    // Minecraft first, as everywhere else.
    components.sort_by_key(|component| component.uid != MINECRAFT_UID);
    Ok(components)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const MMC_PACK: &str = r#"{"formatVersion":1,"components":[
        {"uid":"org.lwjgl3","version":"3.3.3","dependencyOnly":true,"cachedName":"LWJGL 3"},
        {"uid":"net.minecraft","version":"1.20.1","important":true},
        {"uid":"net.fabricmc.intermediary","version":"1.20.1","dependencyOnly":true},
        {"uid":"net.fabricmc.fabric-loader","version":"0.15.11"},
        {"uid":"org.example.custom","version":"1"}]}"#;

    const INSTANCE_CFG: &str = "[General]\nConfigVersion=1.2\nname=My \"Pack\"\nnotes=line one\\nline two\niconKey=grass\n\
        totalTimePlayed=99\nOverrideMemory=true\nMinMemAlloc=1024\nMaxMemAlloc=6144\nOverrideJavaArgs=true\nJvmArgs=-Xss2M\n\
        OverrideJavaLocation=true\nJavaPath=C:\\Java\\bin\\javaw.exe\n";

    fn write_zip(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("test.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, content) in entries {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn open(path: &Path) -> zip::ZipArchive<File> {
        zip::ZipArchive::new(File::open(path).unwrap()).unwrap()
    }

    #[test]
    fn prism_cfg_and_components_map_to_an_instance() {
        let dir = tempfile::tempdir().unwrap();
        let zip = write_zip(
            dir.path(),
            &[
                ("My Pack/instance.cfg", INSTANCE_CFG),
                ("My Pack/mmc-pack.json", MMC_PACK),
                ("My Pack/.minecraft/options.txt", "x"),
            ],
        );
        let imported = read_prism_zip(&mut open(&zip)).unwrap().unwrap();
        assert_eq!(imported.name, "My \"Pack\"");
        let components: Vec<_> = imported.components.iter().map(|c| (c.uid.as_str(), c.version.as_str())).collect();
        assert_eq!(components, [("net.minecraft", "1.20.1"), ("net.fabricmc.fabric-loader", "0.15.11")]);
        assert_eq!(imported.source.game_prefix, "My Pack/.minecraft/");
        assert_eq!(imported.source.notes, "line one\nline two");
        assert_eq!(imported.source.settings.memory, Some(MemorySettings { min_mib: 1024, max_mib: 6144, auto: false }));
        assert_eq!(imported.source.settings.jvm_args.as_deref(), Some("-Xss2M"));
        assert!(imported.source.settings.java.is_none(), "a Java path is machine-specific");
    }

    #[test]
    fn prism_zip_without_overrides_or_dot_minecraft_uses_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let zip = write_zip(
            dir.path(),
            &[("instance.cfg", "name=Plain\nOverrideMemory=false\nMaxMemAlloc=9999\n"), ("mmc-pack.json", MMC_PACK), ("minecraft/a.txt", "x")],
        );
        let imported = read_prism_zip(&mut open(&zip)).unwrap().unwrap();
        assert_eq!(imported.source.game_prefix, "minecraft/");
        assert_eq!(imported.source.settings, InstanceSettings::default());

        let no_minecraft = write_zip(dir.path(), &[("instance.cfg", "name=x\n"), ("mmc-pack.json", r#"{"components":[]}"#)]);
        assert!(read_prism_zip(&mut open(&no_minecraft)).is_err());
        let not_prism = write_zip(dir.path(), &[("readme.txt", "x")]);
        assert!(read_prism_zip(&mut open(&not_prism)).unwrap().is_none());
    }

    #[test]
    fn filter_excludes_paths_with_their_children_and_junk_names() {
        let filter = ExportFilter::new(default_excluded());
        assert!(!filter.allows("logs"));
        assert!(!filter.allows("Logs/latest.log"));
        assert!(!filter.allows("config/.DS_Store"));
        assert!(filter.allows("logs-archive/a.txt"));
        assert!(filter.allows("mods/a.jar"));
        assert!(!ExportFilter::new(["saves/old world".to_owned()]).allows("saves/old world/level.dat"));
    }

    #[cfg(unix)]
    #[test]
    fn export_copies_linked_folders_as_content_and_can_skip_them() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("minecraft");
        let shared = dir.path().join("shared-packs");
        std::fs::create_dir_all(game_dir.join("mods")).unwrap();
        std::fs::create_dir_all(game_dir.join("logs")).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(game_dir.join("mods/a.jar"), "jar").unwrap();
        std::fs::write(game_dir.join("logs/latest.log"), "log").unwrap();
        std::fs::write(shared.join("pack.zip"), "pack").unwrap();
        std::os::unix::fs::symlink(&shared, game_dir.join("resourcepacks")).unwrap();

        let paths = |filter: &ExportFilter| -> Vec<String> {
            collect_files(&game_dir, filter).unwrap().into_iter().map(|file| file.path).collect()
        };
        assert_eq!(paths(&ExportFilter::new(default_excluded())), ["mods/a.jar", "resourcepacks/pack.zip"]);
        let skipped = ExportFilter::new(default_excluded().into_iter().chain(["resourcepacks".to_owned()]));
        assert_eq!(paths(&skipped), ["mods/a.jar"]);
        let listed = list_export_dir(&game_dir, "").unwrap();
        let linked: Vec<_> = listed.iter().filter(|entry| entry.linked).map(|entry| entry.name.as_str()).collect();
        assert_eq!(linked, ["resourcepacks"]);
        assert!(listed.iter().find(|entry| entry.name == "logs").unwrap().default_excluded);
    }

    #[test]
    fn custom_icon_travels_in_our_zip() {
        use crate::instance::{InstanceManager, NewInstance};
        use crate::paths::Paths;

        let dir = tempfile::tempdir().unwrap();
        let manager = InstanceManager::new(Paths::new(dir.path().join("data")));
        let new = NewInstance { name: "Pack".into(), icon: None, group: None, components: Vec::new(), managed_pack: None };
        let id = manager.create(new).unwrap().id;
        let png = b"\x89PNG\r\n\x1a\nimage";
        let instance = manager.set_custom_icon(&id, png).unwrap();

        let zip_path = dir.path().join("out.zip");
        let filter = ExportFilter::new(default_excluded());
        let (instance_dir, game_dir) = (instance.dir(manager.paths()), instance.game_dir(manager.paths()));
        export_instance(&instance, &instance_dir, &game_dir, &zip_path, &filter, &crate::progress::no_progress()).unwrap();

        let imported = read_k0t1k_zip(&mut open(&zip_path)).unwrap().unwrap();
        let icon = imported.source.icon.unwrap();
        assert_eq!(icon, instance.icon);
        assert_eq!(read_custom_icon(&zip_path, &icon).unwrap().as_deref(), Some(&png[..]));
        assert_eq!(read_custom_icon(&zip_path, "grass").unwrap(), None);
    }
}
