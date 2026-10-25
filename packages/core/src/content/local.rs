//! Resources already present in an instance: scan, read embedded metadata, enable/disable, delete.
//! All functions here do blocking file I/O; call them from `spawn_blocking` in async code.

use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::packwiz::{self, PwMod};
use super::{Loader, ResourceKind};
use crate::error::{Error, IoContext, Result};

const DISABLED_SUFFIX: &str = ".disabled";

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
    /// Name on disk, including a trailing `.disabled` when the resource is disabled.
    pub file_name: String,
    pub enabled: bool,
    pub size: u64,
    pub metadata: Option<ResourceMetadata>,
    /// Where the file came from, when a packwiz index describes it.
    pub origin: Option<PwMod>,
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
        let size = if file_type.is_file() {
            entry.metadata().at(&path)?.len()
        } else {
            0
        };
        let metadata = match read_metadata(&path, kind, base_name(&file_name)) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!("cannot read metadata of {}: {error}", path.display());
                None
            }
        };
        resources.push(LocalResource {
            enabled: !file_name.ends_with(DISABLED_SUFFIX),
            size,
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
            let icon_png = std::fs::read(path.join("pack.png")).ok();
            let mcmeta = match std::fs::read(path.join("pack.mcmeta")) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error).at(path.join("pack.mcmeta")),
            };
            Ok(pack_metadata(&mcmeta, stem, icon_png)?)
        }
        _ => Ok(None),
    }
}

/// Reads a whole file of an instance's resource folder, failing when it is larger than `max_bytes`
/// or is a folder.
pub fn read_resource(game_dir: &Path, kind: ResourceKind, file_name: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let path = resource_path(game_dir, kind, file_name)?;
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

/// Renames to or from the `.disabled` suffix and returns the new file name.
/// Directories cannot be toggled; an existing target is an error rather than being overwritten.
pub fn set_enabled(
    game_dir: &Path,
    kind: ResourceKind,
    file_name: &str,
    enabled: bool,
) -> Result<String> {
    let path = resource_path(game_dir, kind, file_name)?;
    if path.is_dir() {
        return Err(Error::Other(format!(
            "{file_name} is a folder and cannot be enabled or disabled"
        )));
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
    let Some(mcmeta) = read_entry(&mut archive, "pack.mcmeta")? else {
        return Ok(None);
    };
    pack_metadata(&mcmeta, stem, read_entry(&mut archive, "pack.png")?)
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
}
