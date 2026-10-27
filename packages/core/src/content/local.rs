//! Resources already present in an instance: scan, read embedded metadata, enable/disable, delete.
//! All functions here do blocking file I/O; call them from `spawn_blocking` in async code.

use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::nested_pack;
use super::packwiz::{self, PwMod};
use super::{Loader, ResourceKind};
use crate::error::{Error, IoContext, Result};

const DISABLED_SUFFIX: &str = ".disabled";
/// Minecraft only treats a folder as a pack when it holds this file, so a folder pack is disabled
/// by renaming it.
const PACK_MCMETA: &str = "pack.mcmeta";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetadata {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub icon_png: Option<Vec<u8>>,
    pub loader: Option<Loader>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalResource {
    /// Name on disk, including a trailing `.disabled` when the resource is disabled. A disabled folder
    /// pack keeps its name: its `pack.mcmeta` is renamed instead.
    pub file_name: String,
    pub enabled: bool,
    /// 0 for a folder.
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: Option<ResourceMetadata>,
    /// Where the file came from, when a packwiz index describes it.
    pub origin: Option<PwMod>,
    /// A resource or data pack whose `pack.mcmeta` is inside extra folders (`"Folder/"`), so Minecraft
    /// does not see it. Name, description and icon are read from there.
    pub nested_root: Option<String>,
}

fn is_pack_kind(kind: ResourceKind) -> bool {
    matches!(kind, ResourceKind::ResourcePack | ResourceKind::DataPack)
}

/// `pack.mcmeta.disabled` of a folder pack.
fn disabled_pack_marker(folder: &Path) -> PathBuf {
    folder.join(format!("{PACK_MCMETA}{DISABLED_SUFFIX}"))
}

fn base_name(file_name: &str) -> &str {
    file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name)
}

/// Resolves a resource name inside its folder, rejecting anything but a plain file name.
fn resource_path(game_dir: &Path, kind: ResourceKind, file_name: &str) -> Result<PathBuf> {
    let mut components = Path::new(file_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(game_dir.join(kind.folder()).join(file_name)),
        _ => Err(Error::Other(format!(
            "{file_name:?} is not a plain file name"
        ))),
    }
}

pub fn scan_resources(game_dir: &Path, kind: ResourceKind) -> Result<Vec<LocalResource>> {
    let folder = game_dir.join(kind.folder());
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(&folder),
    };
    let index = packwiz::list(&packwiz::index_dir(game_dir, kind))?;

    let mut resources = Vec::new();
    for entry in entries {
        let entry = entry.at(&folder)?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let lower = file_name.to_ascii_lowercase();
        let is_housekeeping = file_name.starts_with('.')
            || lower.ends_with(".part")
            || (kind == ResourceKind::ShaderPack
                && (lower.ends_with(".pw.toml") || lower.ends_with(".txt")));
        if is_housekeeping {
            continue;
        }

        let file_type = entry.file_type().at(&path)?;
        let entry_metadata = entry.metadata().at(&path)?;
        let size = if file_type.is_file() { entry_metadata.len() } else { 0 };
        let modified = entry_metadata.modified().ok().map(chrono::DateTime::<chrono::Utc>::from);
        let metadata = match read_metadata(&path, kind, base_name(&file_name)) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!("cannot read metadata of {}: {error}", path.display());
                None
            }
        };
        let nested_root = if is_pack_kind(kind) && (file_type.is_dir() || lower.ends_with(".zip") || lower.ends_with(".zip.disabled")) {
            nested_pack::nested_root_of(&path)
        } else {
            None
        };
        resources.push(LocalResource {
            nested_root,
            enabled: if file_type.is_dir() && is_pack_kind(kind) {
                !disabled_pack_marker(&path).exists() || path.join(PACK_MCMETA).exists()
            } else {
                !file_name.ends_with(DISABLED_SUFFIX)
            },
            size,
            is_dir: file_type.is_dir(),
            modified,
            metadata,
            origin: index
                .iter()
                .find(|index| index.data.filename == base_name(&file_name))
                .map(|index| index.data.clone()),
            file_name,
        });
    }
    resources.sort_by_key(|resource| resource.file_name.to_lowercase());
    Ok(resources)
}

fn read_metadata(
    path: &Path,
    kind: ResourceKind,
    base_name: &str,
) -> Result<Option<ResourceMetadata>> {
    let stem = base_name
        .rsplit_once('.')
        .map_or(base_name, |(stem, _)| stem);
    match kind {
        ResourceKind::Mod if path.is_file() => parse_mod_jar(std::fs::File::open(path).at(path)?),
        ResourceKind::ResourcePack | ResourceKind::DataPack if path.is_file() => {
            parse_pack(std::fs::File::open(path).at(path)?, stem)
        }
        ResourceKind::ResourcePack | ResourceKind::DataPack => {
            let nested = nested_pack::folder_nested_root(path).unwrap_or_default();
            let path = &path.join(nested);
            let icon_png = std::fs::read(path.join("pack.png")).ok();
            let mcmeta = match std::fs::read(path.join(PACK_MCMETA)) {
                Ok(bytes) => bytes,
                // A disabled folder pack still shows its description.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::read(disabled_pack_marker(path)) {
                        Ok(bytes) => bytes,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                        Err(error) => return Err(error).at(disabled_pack_marker(path)),
                    }
                }
                Err(error) => return Err(error).at(path.join(PACK_MCMETA)),
            };
            Ok(pack_metadata(&mcmeta, stem, icon_png)?)
        }
        _ => Ok(None),
    }
}

const MAX_PACK_DEPTH: usize = 32;

/// Zips a folder pack in memory. A disabled pack's `pack.mcmeta.disabled` is stored as `pack.mcmeta`.
/// Symbolic links are skipped; the sum of the file sizes may not exceed `max_bytes`.
fn zip_folder_pack(root: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let has_active_mcmeta = root.join(PACK_MCMETA).is_file();
    let mut total = 0u64;
    let mut pending = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = pending.pop() {
        if depth > MAX_PACK_DEPTH {
            return Err(Error::Other(format!("{} is nested too deeply", root.display())));
        }
        for entry in std::fs::read_dir(&dir).at(&dir)? {
            let entry = entry.at(&dir)?;
            let path = entry.path();
            let file_type = entry.file_type().at(&path)?;
            let mut name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if file_type.is_dir() {
                pending.push((path, format!("{name}/"), depth + 1));
            } else if file_type.is_file() {
                if name == format!("{PACK_MCMETA}{DISABLED_SUFFIX}") {
                    if has_active_mcmeta {
                        continue;
                    }
                    name = PACK_MCMETA.to_owned();
                }
                total += entry.metadata().at(&path)?.len();
                if total > max_bytes {
                    return Err(Error::Other(format!("{} is larger than {max_bytes} bytes", root.display())));
                }
                let bytes = std::fs::read(&path).at(&path)?;
                writer.start_file(name, options)?;
                writer.write_all(&bytes).at(&path)?;
            }
        }
    }
    Ok(writer.finish()?.into_inner())
}

/// The format range declared by a folder pack's `pack.mcmeta` (or `pack.mcmeta.disabled`); `None`
/// when it has none or it is unreadable.
pub fn folder_declared_range(folder: &Path) -> Option<(crate::packformat::PackFormat, crate::packformat::PackFormat)> {
    let bytes = std::fs::read(folder.join(PACK_MCMETA)).or_else(|_| std::fs::read(disabled_pack_marker(folder))).ok()?;
    crate::packformat::declared_range(&bytes)
}

/// Reads a whole file of an instance's resource folder, failing when it is larger than `max_bytes`.
/// A folder pack is read as an in-memory zip of its contents, so plugins treat it like a zip pack.
pub fn read_resource(game_dir: &Path, kind: ResourceKind, file_name: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let path = resource_path(game_dir, kind, file_name)?;
    if path.is_dir() && is_pack_kind(kind) {
        return zip_folder_pack(&path, max_bytes);
    }
    let size = std::fs::metadata(&path).at(&path)?.len();
    if !path.is_file() || size > max_bytes {
        return Err(Error::Other(format!("{file_name} is not a file or is larger than {max_bytes} bytes")));
    }
    std::fs::read(&path).at(&path)
}

/// Saves `bytes` as `file_name` in the resource folder without replacing anything: when the name is
/// taken, ` (2)`, ` (3)`... is inserted before the extension. Returns the name used.
pub fn write_resource_new(game_dir: &Path, kind: ResourceKind, file_name: &str, bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    let (stem, extension) = file_name.rsplit_once('.').map_or((file_name, ""), |(stem, extension)| (stem, extension));
    for attempt in 1..100 {
        let candidate = match (attempt, extension) {
            (1, _) => file_name.to_owned(),
            (_, "") => format!("{stem} ({attempt})"),
            _ => format!("{stem} ({attempt}).{extension}"),
        };
        let path = resource_path(game_dir, kind, &candidate)?;
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    // Do not leave a truncated file that would look like a pack.
                    let _ = std::fs::remove_file(&path);
                    return Err(error).at(&path);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).at(&path),
        }
    }
    Err(Error::Other(format!("too many files named like {file_name}")))
}

/// Renames to or from the `.disabled` suffix and returns the new file name. A resource or data pack
/// folder is toggled by renaming its `pack.mcmeta` instead and keeps its name; other folders cannot be
/// toggled. An existing target is an error rather than being overwritten.
pub fn set_enabled(
    game_dir: &Path,
    kind: ResourceKind,
    file_name: &str,
    enabled: bool,
) -> Result<String> {
    let path = resource_path(game_dir, kind, file_name)?;
    if path.is_dir() {
        if !is_pack_kind(kind) {
            return Err(Error::Other(format!(
                "{file_name} is a folder and cannot be enabled or disabled"
            )));
        }
        let (active, inactive) = (path.join(PACK_MCMETA), disabled_pack_marker(&path));
        let (from, to) = if enabled { (inactive, active) } else { (active, inactive) };
        if to.exists() {
            if enabled || !from.exists() {
                return Ok(file_name.to_owned());
            }
            // Both files exist: the live pack.mcmeta is current, the disabled copy is stale. Keep the
            // stale one under another name instead of failing forever or deleting it.
            let backup = to.with_file_name(format!("{PACK_MCMETA}{DISABLED_SUFFIX}.old"));
            std::fs::rename(&to, &backup).at(&backup)?;
        }
        if !from.exists() {
            return Err(Error::Other(format!("{file_name} has no {PACK_MCMETA}")));
        }
        std::fs::rename(&from, &to).at(&to)?;
        return Ok(file_name.to_owned());
    }
    let is_disabled = file_name.ends_with(DISABLED_SUFFIX);
    if is_disabled != enabled {
        return Ok(file_name.to_owned());
    }
    let new_name = if enabled {
        base_name(file_name).to_owned()
    } else {
        format!("{file_name}{DISABLED_SUFFIX}")
    };
    let target = resource_path(game_dir, kind, &new_name)?;
    if target.exists() {
        return Err(Error::Other(format!("{new_name} already exists")));
    }
    std::fs::rename(&path, &target).at(&target)?;
    Ok(new_name)
}

/// Makes a resource or data pack with an extra top folder loadable: a zip is rewritten with the folder
/// as its root, a folder pack has its content moved up. Returns whether the pack needed fixing; on
/// failure the pack is unchanged.
pub fn fix_nested_pack(game_dir: &Path, kind: ResourceKind, file_name: &str) -> Result<bool> {
    if !is_pack_kind(kind) {
        return Err(Error::Other(format!("{file_name} is not a pack")));
    }
    let path = resource_path(game_dir, kind, file_name)?;
    if path.is_dir() {
        nested_pack::fix_folder(&path)
    } else {
        nested_pack::fix_zip(&path)
    }
}

/// Deletes the file (or folder) together with its packwiz index entry.
pub fn delete_resource(game_dir: &Path, kind: ResourceKind, file_name: &str) -> Result<()> {
    let path = resource_path(game_dir, kind, file_name)?;
    if path.is_dir() {
        std::fs::remove_dir_all(&path).at(&path)?;
    } else {
        std::fs::remove_file(&path).at(&path)?;
    }
    for entry in packwiz::list(&packwiz::index_dir(game_dir, kind))? {
        if entry.data.filename == base_name(file_name) {
            std::fs::remove_file(&entry.path).at(&entry.path)?;
        }
    }
    Ok(())
}

type Parsed = Option<(ResourceMetadata, Option<String>)>;
type Parser = fn(&[u8], &Manifest) -> Result<Parsed>;

/// Reads mod metadata from a jar. The first recognised descriptor wins; `None` when there is none.
pub fn parse_mod_jar(reader: impl Read + Seek) -> Result<Option<ResourceMetadata>> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let parsers: [(&str, Parser); 5] = [
        ("quilt.mod.json", parse_quilt),
        ("fabric.mod.json", parse_fabric),
        ("META-INF/neoforge.mods.toml", |text, manifest| {
            parse_mods_toml(text, manifest, Loader::NeoForge)
        }),
        ("META-INF/mods.toml", |text, manifest| {
            parse_mods_toml(text, manifest, Loader::Forge)
        }),
        ("mcmod.info", parse_mcmod_info),
    ];
    let manifest = Manifest(read_entry(&mut archive, "META-INF/MANIFEST.MF")?);
    for (entry, parse) in parsers {
        let Some(bytes) = read_entry(&mut archive, entry)? else {
            continue;
        };
        if let Some((mut metadata, icon)) = parse(&bytes, &manifest)? {
            if let Some(icon) = icon {
                metadata.icon_png = read_entry(&mut archive, icon.trim_start_matches('/'))?;
            }
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

fn parse_pack(reader: impl Read + Seek, stem: &str) -> Result<Option<ResourceMetadata>> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let mut names = Vec::with_capacity(archive.len());
    for name in archive.file_names() {
        names.push(name?.into_owned());
    }
    let root = nested_pack::zip_nested_root(&names).unwrap_or_default();
    let Some(mcmeta) = read_entry(&mut archive, &format!("{root}pack.mcmeta"))? else {
        return Ok(None);
    };
    pack_metadata(&mcmeta, stem, read_entry(&mut archive, &format!("{root}pack.png"))?)
}

fn pack_metadata(
    mcmeta: &[u8],
    stem: &str,
    icon_png: Option<Vec<u8>>,
) -> Result<Option<ResourceMetadata>> {
    let json: Value = serde_json::from_slice(mcmeta)?;
    Ok(Some(ResourceMetadata {
        id: stem.to_owned(),
        name: stem.to_owned(),
        description: json["pack"]
            .get("description")
            .map(text_component)
            .filter(|text| !text.is_empty()),
        icon_png,
        ..ResourceMetadata::default()
    }))
}

/// Flattens a Minecraft text component (string, `{text, extra}` object or array) to plain text.
fn text_component(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().map(text_component).collect(),
        Value::Object(object) => {
            let own = object.get("text").map(text_component).unwrap_or_default();
            own + &object.get("extra").map(text_component).unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).at(name)?;
    Ok(Some(bytes))
}

/// `META-INF/MANIFEST.MF`, only used to resolve `${file.jarVersion}`.
struct Manifest(Option<Vec<u8>>);

impl Manifest {
    fn implementation_version(&self) -> Option<String> {
        let text = String::from_utf8_lossy(self.0.as_deref()?);
        text.lines()
            .find_map(|line| line.strip_prefix("Implementation-Version:"))
            .map(|version| version.trim().to_owned())
    }
}

fn string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// Fabric and Quilt icons are a path, or an object keyed by pixel size where the largest wins.
fn icon_path(icon: &Value) -> Option<String> {
    match icon {
        Value::String(path) => Some(path.clone()),
        Value::Object(sizes) => sizes
            .iter()
            .filter_map(|(size, path)| {
                Some((size.split('x').next()?.parse::<u32>().ok()?, path.as_str()?))
            })
            .max_by_key(|(size, _)| *size)
            .map(|(_, path)| path.to_owned()),
        _ => None,
    }
}

fn parse_fabric(bytes: &[u8], _: &Manifest) -> Result<Parsed> {
    let json: Value = serde_json::from_slice(bytes)?;
    let Some(id) = string(&json["id"]) else {
        return Ok(None);
    };
    let authors = json["authors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|author| string(author).or_else(|| string(&author["name"])))
        .collect();
    let metadata = ResourceMetadata {
        name: string(&json["name"]).unwrap_or_else(|| id.clone()),
        id,
        version: string(&json["version"]),
        authors,
        description: string(&json["description"]),
        icon_png: None,
        loader: Some(Loader::Fabric),
    };
    let has_icon = json["schemaVersion"].as_u64().unwrap_or(0) >= 1;
    Ok(Some((
        metadata,
        has_icon.then(|| icon_path(&json["icon"])).flatten(),
    )))
}

fn parse_quilt(bytes: &[u8], _: &Manifest) -> Result<Parsed> {
    let json: Value = serde_json::from_slice(bytes)?;
    let loader = &json["quilt_loader"];
    let Some(id) = string(&loader["id"]) else {
        return Ok(None);
    };
    let info = &loader["metadata"];
    let metadata = ResourceMetadata {
        name: string(&info["name"]).unwrap_or_else(|| id.clone()),
        id,
        version: string(&loader["version"]),
        authors: info["contributors"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name.clone())
            .collect(),
        description: string(&info["description"]),
        icon_png: None,
        loader: Some(Loader::Quilt),
    };
    Ok(Some((metadata, icon_path(&info["icon"]))))
}

fn parse_mods_toml(bytes: &[u8], manifest: &Manifest, loader: Loader) -> Result<Parsed> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| Error::Other(format!("mods.toml is not UTF-8: {error}")))?;
    let root: toml::Table = toml::from_str(text)
        .map_err(|error| Error::Other(format!("invalid mods.toml: {error}")))?;
    let Some(entry) = root
        .get("mods")
        .and_then(|mods| mods.as_array())
        .and_then(|mods| mods.first())
    else {
        return Ok(None);
    };
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let Some(id) = text("modId") else {
        return Ok(None);
    };
    let version = text("version").map(|version| {
        if version == "${file.jarVersion}" {
            manifest
                .implementation_version()
                .unwrap_or_else(|| "NONE".to_owned())
        } else {
            version
        }
    });
    let authors = text("authors")
        .map(|authors| {
            authors
                .split(',')
                .map(|author| author.trim().to_owned())
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let logo = text("logoFile").or_else(|| {
        root.get("logoFile")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    });
    let metadata = ResourceMetadata {
        name: text("displayName").unwrap_or_else(|| id.clone()),
        id,
        version,
        authors,
        description: text("description"),
        icon_png: None,
        loader: Some(loader),
    };
    Ok(Some((metadata, logo)))
}

fn parse_mcmod_info(bytes: &[u8], _: &Manifest) -> Result<Parsed> {
    let json: Value = serde_json::from_slice(bytes)?;
    let list = match &json {
        Value::Array(_) => &json,
        _ => json
            .get("modList")
            .or_else(|| json.get("modlist"))
            .unwrap_or(&Value::Null),
    };
    let Some(entry) = list.get(0) else {
        return Ok(None);
    };
    let (Some(id), Some(name)) = (string(&entry["modid"]), string(&entry["name"])) else {
        return Ok(None);
    };
    if name == "Example Mod" {
        return Ok(None);
    }
    let authors = ["authorList", "authors"]
        .iter()
        .find_map(|key| entry[*key].as_array())
        .into_iter()
        .flatten()
        .filter_map(string)
        .collect();
    let metadata = ResourceMetadata {
        id,
        name,
        version: string(&entry["version"]),
        authors,
        description: string(&entry["description"]),
        icon_png: None,
        loader: Some(Loader::Forge),
    };
    Ok(Some((
        metadata,
        string(&entry["logoFile"]).filter(|path| !path.is_empty()),
    )))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> Cursor<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        Cursor::new(writer.finish().unwrap().into_inner())
    }

    fn mod_of(entries: &[(&str, &[u8])]) -> ResourceMetadata {
        parse_mod_jar(zip_of(entries)).unwrap().unwrap()
    }

    #[test]
    fn fabric_mod_with_sized_icons_and_mixed_authors() {
        let json = br#"{"schemaVersion":1,"id":"sodium","version":"0.5.13","name":"Sodium","description":"fast",
            "authors":["jellysquid3",{"name":"IMS"}],"icon":{"32":"small.png","128":"big.png"}}"#;
        let metadata = mod_of(&[
            ("fabric.mod.json", json),
            ("small.png", b"S"),
            ("big.png", b"B"),
        ]);
        assert_eq!(metadata.id, "sodium");
        assert_eq!(metadata.authors, ["jellysquid3", "IMS"]);
        assert_eq!(metadata.icon_png.as_deref(), Some(&b"B"[..]));
        assert_eq!(metadata.loader, Some(Loader::Fabric));
    }

    #[test]
    fn fabric_name_falls_back_to_id() {
        let metadata = mod_of(&[(
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"x","version":"1"}"#,
        )]);
        assert_eq!(metadata.name, "x");
        assert_eq!(metadata.icon_png, None);
    }

    #[test]
    fn quilt_mod_reads_nested_loader_section() {
        let json = br#"{"quilt_loader":{"id":"qsl","version":"2.0","metadata":{"name":"QSL","description":"d",
            "contributors":{"Alice":"Owner"},"icon":"icon.png"}}}"#;
        let metadata = mod_of(&[("quilt.mod.json", json), ("icon.png", b"I")]);
        assert_eq!(
            (metadata.id.as_str(), metadata.loader),
            ("qsl", Some(Loader::Quilt))
        );
        assert_eq!(metadata.authors, ["Alice"]);
        assert_eq!(metadata.icon_png.as_deref(), Some(&b"I"[..]));
    }

    #[test]
    fn forge_jar_version_comes_from_manifest() {
        let toml = br#"
logoFile = "logo.png"
[[mods]]
modId = "jei"
version = "${file.jarVersion}"
displayName = "Just Enough Items"
authors = "mezz, others"
"#;
        let manifest = b"Manifest-Version: 1.0\nImplementation-Version: 15.2.0\n";
        let metadata = mod_of(&[
            ("META-INF/mods.toml", toml),
            ("META-INF/MANIFEST.MF", manifest),
            ("logo.png", b"L"),
        ]);
        assert_eq!(metadata.version.as_deref(), Some("15.2.0"));
        assert_eq!(metadata.authors, ["mezz", "others"]);
        assert_eq!(metadata.icon_png.as_deref(), Some(&b"L"[..]));
        assert_eq!(metadata.loader, Some(Loader::Forge));
    }

    #[test]
    fn missing_manifest_version_becomes_none_marker_and_neoforge_is_detected() {
        let toml = b"[[mods]]\nmodId = \"m\"\nversion = \"${file.jarVersion}\"\n";
        let metadata = mod_of(&[("META-INF/neoforge.mods.toml", toml)]);
        assert_eq!(metadata.version.as_deref(), Some("NONE"));
        assert_eq!(metadata.loader, Some(Loader::NeoForge));
    }

    #[test]
    fn legacy_mcmod_info_array_and_example_mod_is_ignored() {
        let info =
            br#"[{"modid":"old","name":"Old Mod","version":"1","authorList":["A"],"logoFile":""}]"#;
        let metadata = mod_of(&[("mcmod.info", info)]);
        assert_eq!(
            (metadata.id.as_str(), metadata.authors),
            ("old", vec!["A".to_owned()])
        );

        let wrapped = br#"{"modList":[{"modid":"x","name":"Example Mod"}]}"#;
        assert!(
            parse_mod_jar(zip_of(&[("mcmod.info", wrapped)]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn jar_without_descriptor_has_no_metadata() {
        assert!(
            parse_mod_jar(zip_of(&[("a.class", b"x")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pack_description_may_be_a_text_component() {
        let mcmeta = br#"{"pack":{"pack_format":34,"description":[{"text":"Hello "},{"text":"World","extra":["!"]}]}}"#;
        let metadata = parse_pack(
            zip_of(&[("pack.mcmeta", mcmeta), ("pack.png", b"P")]),
            "My Pack",
        )
        .unwrap()
        .unwrap();
        assert_eq!(metadata.description.as_deref(), Some("Hello World!"));
        assert_eq!(metadata.icon_png.as_deref(), Some(&b"P"[..]));
        assert_eq!(metadata.name, "My Pack");
    }

    #[test]
    fn scan_toggle_and_delete() {
        let game_dir = std::env::temp_dir().join(format!("k0t1k-local-{}", uuid::Uuid::new_v4()));
        let mods = game_dir.join("mods");
        std::fs::create_dir_all(mods.join(".index")).unwrap();
        let jar = zip_of(&[(
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"a","version":"1"}"#,
        )])
        .into_inner();
        std::fs::write(mods.join("a.jar"), &jar).unwrap();
        std::fs::write(mods.join("b.jar.disabled"), b"not a zip").unwrap();
        std::fs::write(mods.join("c.jar.part"), b"partial").unwrap();
        let index = PwMod {
            filename: "a.jar".into(),
            ..PwMod::default()
        };
        packwiz::write(&mods.join(".index"), "a", &index).unwrap();

        let found = scan_resources(&game_dir, ResourceKind::Mod).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].enabled && found[0].metadata.is_some() && found[0].origin.is_some());
        assert!(!found[1].enabled && found[1].metadata.is_none());

        assert_eq!(
            set_enabled(&game_dir, ResourceKind::Mod, "a.jar", false).unwrap(),
            "a.jar.disabled"
        );
        assert_eq!(
            set_enabled(&game_dir, ResourceKind::Mod, "a.jar.disabled", true).unwrap(),
            "a.jar"
        );
        assert!(set_enabled(&game_dir, ResourceKind::Mod, "../a.jar", true).is_err());

        delete_resource(&game_dir, ResourceKind::Mod, "a.jar").unwrap();
        assert!(!mods.join("a.jar").exists());
        assert!(packwiz::list(&mods.join(".index")).unwrap().is_empty());
        std::fs::remove_dir_all(&game_dir).unwrap();
    }

    #[test]
    fn folder_pack_is_read_as_a_zip() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("resourcepacks/Folder");
        std::fs::create_dir_all(pack.join("assets/minecraft/textures")).unwrap();
        std::fs::write(pack.join("pack.mcmeta.disabled"), br#"{"pack":{"pack_format":15}}"#).unwrap();
        std::fs::write(pack.join("assets/minecraft/textures/a.png"), b"PNG").unwrap();
        assert_eq!(folder_declared_range(&pack), Some(((15, 0), (15, 0))));

        let bytes = read_resource(dir.path(), ResourceKind::ResourcePack, "Folder", 1 << 20).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<_> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["assets/minecraft/textures/a.png", "pack.mcmeta"]);
        let mut content = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("pack.mcmeta").unwrap(), &mut content).unwrap();
        assert!(content.contains("15"));
        assert!(read_resource(dir.path(), ResourceKind::ResourcePack, "Folder", 4).is_err());
    }

    #[test]
    fn folder_pack_is_toggled_through_its_pack_mcmeta() {
        let dir = tempfile::tempdir().unwrap();
        let kind = ResourceKind::ResourcePack;
        let pack = dir.path().join("resourcepacks/§9s§3s §bPACK");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), br#"{"pack":{"description":"hi"}}"#).unwrap();
        let name = "§9s§3s §bPACK";

        assert!(scan_resources(dir.path(), kind).unwrap()[0].enabled);
        assert_eq!(set_enabled(dir.path(), kind, name, false).unwrap(), name);
        assert!(!pack.join("pack.mcmeta").exists() && pack.join("pack.mcmeta.disabled").exists());
        let found = scan_resources(dir.path(), kind).unwrap();
        assert!(!found[0].enabled);
        assert_eq!(found[0].metadata.as_ref().unwrap().description.as_deref(), Some("hi"));

        // Repeating is a no-op; enabling restores the file.
        assert_eq!(set_enabled(dir.path(), kind, name, false).unwrap(), name);
        assert_eq!(set_enabled(dir.path(), kind, name, true).unwrap(), name);
        assert!(pack.join("pack.mcmeta").exists() && scan_resources(dir.path(), kind).unwrap()[0].enabled);

        // A stale pack.mcmeta.disabled next to a live pack.mcmeta is set aside, the live file wins.
        std::fs::write(pack.join("pack.mcmeta.disabled"), b"stale").unwrap();
        assert_eq!(set_enabled(dir.path(), kind, name, false).unwrap(), name);
        assert!(!pack.join("pack.mcmeta").exists());
        assert_eq!(std::fs::read(pack.join("pack.mcmeta.disabled.old")).unwrap(), b"stale");
        assert_eq!(scan_resources(dir.path(), kind).unwrap()[0].metadata.as_ref().unwrap().description.as_deref(), Some("hi"));
        assert_eq!(set_enabled(dir.path(), kind, name, true).unwrap(), name);

        // Folders without a pack.mcmeta, and other kinds of folders, cannot be toggled.
        std::fs::remove_file(pack.join("pack.mcmeta")).unwrap();
        assert!(set_enabled(dir.path(), kind, name, false).is_err());
        std::fs::create_dir_all(dir.path().join("mods/folder")).unwrap();
        assert!(set_enabled(dir.path(), ResourceKind::Mod, "folder", false).is_err());
    }

    #[test]
    fn new_files_never_replace_existing_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("resourcepacks")).unwrap();
        let kind = ResourceKind::ResourcePack;
        assert_eq!(write_resource_new(dir.path(), kind, "a (1.21).zip", b"1").unwrap(), "a (1.21).zip");
        assert_eq!(write_resource_new(dir.path(), kind, "a (1.21).zip", b"2").unwrap(), "a (1.21) (2).zip");
        assert_eq!(std::fs::read(dir.path().join("resourcepacks/a (1.21).zip")).unwrap(), b"1");
        assert_eq!(read_resource(dir.path(), kind, "a (1.21) (2).zip", 10).unwrap(), b"2");
        assert!(read_resource(dir.path(), kind, "a (1.21) (2).zip", 0).is_err());
        assert!(write_resource_new(dir.path(), kind, "../x.zip", b"3").is_err());
    }

    #[test]
    fn nested_packs_are_listed_with_their_metadata_and_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let kind = ResourceKind::ResourcePack;
        let packs = dir.path().join("resourcepacks");
        std::fs::create_dir_all(packs.join("folder/Inner")).unwrap();
        std::fs::write(packs.join("folder/Inner/pack.mcmeta"), br#"{"pack":{"description":"in folder"}}"#).unwrap();
        std::fs::write(
            packs.join("a.zip.disabled"),
            zip_of(&[("Top/pack.mcmeta", br#"{"pack":{"description":"in zip"}}"#), ("Top/pack.png", b"P")]).into_inner(),
        )
        .unwrap();
        std::fs::write(packs.join("ok.zip"), zip_of(&[("pack.mcmeta", b"{}")]).into_inner()).unwrap();

        let found = scan_resources(dir.path(), kind).unwrap();
        let by_name = |name: &str| found.iter().find(|resource| resource.file_name == name).unwrap();
        assert_eq!(by_name("a.zip.disabled").nested_root.as_deref(), Some("Top/"));
        assert_eq!(by_name("a.zip.disabled").metadata.as_ref().unwrap().description.as_deref(), Some("in zip"));
        assert_eq!(by_name("a.zip.disabled").metadata.as_ref().unwrap().icon_png.as_deref(), Some(&b"P"[..]));
        assert_eq!(by_name("folder").nested_root.as_deref(), Some("Inner/"));
        assert_eq!(by_name("folder").metadata.as_ref().unwrap().description.as_deref(), Some("in folder"));
        assert_eq!(by_name("ok.zip").nested_root, None);

        assert!(fix_nested_pack(dir.path(), kind, "a.zip.disabled").unwrap());
        assert!(fix_nested_pack(dir.path(), kind, "folder").unwrap());
        assert!(!fix_nested_pack(dir.path(), kind, "ok.zip").unwrap());
        assert!(fix_nested_pack(dir.path(), ResourceKind::Mod, "ok.zip").is_err());
        let found = scan_resources(dir.path(), kind).unwrap();
        assert!(found.iter().all(|resource| resource.nested_root.is_none()));
        assert_eq!(found[0].file_name, "a.zip.disabled");
        assert_eq!(found[0].metadata.as_ref().unwrap().description.as_deref(), Some("in zip"));
        assert!(packs.join("folder/pack.mcmeta").is_file());
    }
}
