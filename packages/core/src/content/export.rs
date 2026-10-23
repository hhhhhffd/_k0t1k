//! Exporting an instance as a Modrinth `.mrpack` or a CurseForge zip. Mods and packs that a platform
//! recognises by hash become download entries; everything else (config, unknown mods, saves the user
//! left selected) is packed into `overrides/`.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha512};

use super::modpack::{
    FABRIC_UID, FORGE_UID, MrpackEnv, MrpackFile, NEOFORGE_UID, QUILT_UID,
};
use super::modrinth::HashAlgorithm;
use super::{Clients, Hashes, ResourceKind, murmur2, packwiz};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::error::{Error, IoContext, Result};
use crate::instance::archive::{ExportFile, ExportFilter, add_file, collect_files, zip_options};
use crate::progress::{ProgressEvent, ProgressFn};

const TASK: &str = "export";
const CONTENT_FOLDERS: [(&str, ResourceKind); 5] = [
    ("mods", ResourceKind::Mod),
    ("coremods", ResourceKind::Mod),
    ("resourcepacks", ResourceKind::ResourcePack),
    ("texturepacks", ResourceKind::ResourcePack),
    ("shaderpacks", ResourceKind::ShaderPack),
];
const CONTENT_EXTENSIONS: [&str; 3] = ["jar", "litemod", "zip"];
const DISABLED_SUFFIX: &str = ".disabled";

/// What the user chose for the pack.
pub struct PackExportRequest<'a> {
    pub game_dir: &'a Path,
    pub destination: &'a Path,
    pub name: &'a str,
    pub version: &'a str,
    pub summary: Option<&'a str>,
    /// The instance's components: Minecraft and its loader.
    pub components: &'a [ComponentSpec],
    pub filter: &'a ExportFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackExportReport {
    /// Files the pack downloads from the platform.
    pub listed: usize,
    /// Files packed into the archive's `overrides/`, because the platform does not know them.
    pub overrides: usize,
}

/// A mod, resource pack or shader pack file: the kind of file the platforms can identify.
struct Candidate {
    file: ExportFile,
    disabled: bool,
    /// `client`, `server` or `both` from the packwiz index, when the file has one.
    side: Option<String>,
    sha512: String,
    fingerprint: u32,
}

struct Scan {
    candidates: Vec<Candidate>,
    others: Vec<ExportFile>,
}

/// A file as the platform knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    sha1: String,
    sha512: String,
    url: String,
    size: u64,
}

fn content_extension_ok(path: &str) -> bool {
    let name = path.strip_suffix(DISABLED_SUFFIX).unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        CONTENT_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known))
    })
}

fn is_index_file(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".pw.toml")
        || path.split('/').any(|segment| segment == ".index")
}

/// Splits the allowed game dir files into identifiable content (hashed) and the rest. Packwiz index
/// files are dropped: the importing side writes its own.
fn scan(game_dir: &Path, filter: &ExportFilter, progress: &ProgressFn) -> Result<Scan> {
    let mut sides: HashMap<(&str, String), String> = HashMap::new();
    for (folder, kind) in CONTENT_FOLDERS {
        for entry in packwiz::list(&packwiz::index_dir(game_dir, kind))? {
            sides.insert((folder, entry.data.filename), entry.data.side);
        }
    }
    let files = collect_files(game_dir, filter)?;
    let total = files.len() as u64;
    let mut scan = Scan { candidates: Vec::new(), others: Vec::new() };
    for (done, file) in files.into_iter().enumerate() {
        progress(ProgressEvent {
            task: TASK.to_owned(),
            stage: "Hashing files".to_owned(),
            done: done as u64,
            total,
        });
        let folder = file.path.split('/').next().unwrap_or_default();
        let is_content = CONTENT_FOLDERS.iter().any(|(name, _)| *name == folder) && content_extension_ok(&file.path);
        if is_content {
            let bytes = std::fs::read(&file.absolute).at(&file.absolute)?;
            let name = file.path.rsplit('/').next().unwrap_or_default();
            let disabled = name.ends_with(DISABLED_SUFFIX);
            let plain_name = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name).to_owned();
            let side = CONTENT_FOLDERS
                .iter()
                .find(|(known, _)| *known == folder)
                .and_then(|(known, _)| sides.get(&(*known, plain_name)))
                .filter(|side| !side.is_empty())
                .cloned();
            scan.candidates.push(Candidate {
                sha512: hex::encode(Sha512::digest(&bytes)),
                fingerprint: murmur2::fingerprint(&bytes),
                file,
                disabled,
                side,
            });
        } else if !is_index_file(&file.path) {
            scan.others.push(file);
        }
    }
    Ok(scan)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MrpackIndexOut {
    format_version: u32,
    game: &'static str,
    version_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    files: Vec<MrpackFile>,
    dependencies: BTreeMap<String, String>,
}

fn mrpack_dependencies(components: &[ComponentSpec]) -> Result<BTreeMap<String, String>> {
    let mut dependencies = BTreeMap::new();
    for component in components {
        let key = match component.uid.as_str() {
            MINECRAFT_UID => "minecraft",
            FABRIC_UID => "fabric-loader",
            QUILT_UID => "quilt-loader",
            FORGE_UID => "forge",
            NEOFORGE_UID => "neoforge",
            _ => continue,
        };
        dependencies.insert(key.to_owned(), component.version.clone());
    }
    if !dependencies.contains_key("minecraft") {
        return Err(Error::Other("the instance has no Minecraft component".into()));
    }
    Ok(dependencies)
}

/// The mrpack index for `candidates` and the indices of those the platform did not resolve (they go
/// to overrides). `resolved` is keyed by the candidate's sha512.
fn build_mrpack_index(
    request: &PackExportRequest<'_>,
    candidates: &[Candidate],
    resolved: &HashMap<String, Resolved>,
) -> Result<(MrpackIndexOut, Vec<usize>)> {
    let mut files = Vec::new();
    let mut unresolved = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(found) = resolved.get(&candidate.sha512) else {
            unresolved.push(index);
            continue;
        };
        // A disabled file becomes an optional one under its normal name.
        let path = candidate.file.path.strip_suffix(DISABLED_SUFFIX).unwrap_or(&candidate.file.path).to_owned();
        let client = if candidate.disabled { "optional" } else { "required" };
        let server = match (candidate.disabled, candidate.side.as_deref()) {
            (_, Some("client")) => "unsupported",
            (true, _) => "optional",
            (false, _) => "required",
        };
        files.push(MrpackFile {
            path,
            hashes: Hashes { sha1: Some(found.sha1.clone()), sha512: Some(found.sha512.clone()) },
            downloads: vec![found.url.clone()],
            file_size: found.size,
            env: Some(MrpackEnv { client: Some(client.to_owned()), server: Some(server.to_owned()) }),
        });
    }
    let index = MrpackIndexOut {
        format_version: 1,
        game: "minecraft",
        version_id: request.version.to_owned(),
        name: request.name.to_owned(),
        summary: request.summary.filter(|summary| !summary.is_empty()).map(str::to_owned),
        files,
        dependencies: mrpack_dependencies(request.components)?,
    };
    Ok((index, unresolved))
}

#[derive(Serialize)]
struct CurseForgeLoaderOut {
    id: String,
    primary: bool,
}

#[derive(Serialize)]
struct CurseForgeMinecraftOut {
    version: String,
    #[serde(rename = "modLoaders")]
    mod_loaders: Vec<CurseForgeLoaderOut>,
}

#[derive(Serialize)]
struct CurseForgeFileOut {
    #[serde(rename = "projectID")]
    project_id: u64,
    #[serde(rename = "fileID")]
    file_id: u64,
    required: bool,
}

#[derive(Serialize)]
struct CurseForgeManifestOut {
    minecraft: CurseForgeMinecraftOut,
    #[serde(rename = "manifestType")]
    manifest_type: &'static str,
    #[serde(rename = "manifestVersion")]
    manifest_version: u32,
    name: String,
    version: String,
    author: &'static str,
    files: Vec<CurseForgeFileOut>,
    overrides: &'static str,
}

/// The CurseForge manifest. `resolved` maps a candidate's fingerprint to `(project id, file id)`.
fn build_curseforge_manifest(
    request: &PackExportRequest<'_>,
    candidates: &[Candidate],
    resolved: &HashMap<u32, (u64, u64)>,
) -> Result<(CurseForgeManifestOut, Vec<usize>)> {
    let dependencies = mrpack_dependencies(request.components)?;
    let minecraft = dependencies["minecraft"].clone();
    let mod_loaders = ["fabric-loader", "quilt-loader", "forge", "neoforge"]
        .iter()
        .zip(["fabric", "quilt", "forge", "neoforge"])
        .find_map(|(key, prefix)| {
            dependencies.get(*key).map(|version| CurseForgeLoaderOut { id: format!("{prefix}-{version}"), primary: true })
        })
        .into_iter()
        .collect();

    let mut files = Vec::new();
    let mut unresolved = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        match resolved.get(&candidate.fingerprint) {
            Some(&(project_id, file_id)) => files.push(CurseForgeFileOut { project_id, file_id, required: !candidate.disabled }),
            None => unresolved.push(index),
        }
    }
    let manifest = CurseForgeManifestOut {
        minecraft: CurseForgeMinecraftOut { version: minecraft, mod_loaders },
        manifest_type: "minecraftModpack",
        manifest_version: 1,
        name: request.name.to_owned(),
        version: request.version.to_owned(),
        author: "_k0t1k",
        files,
        overrides: "overrides",
    };
    Ok((manifest, unresolved))
}

/// Writes the archive: the manifest and `overrides/<path>` for each override file, next to the
/// destination first and renamed when complete.
fn write_pack(
    destination: &Path,
    manifest_name: &str,
    manifest: &[u8],
    overrides: &[&ExportFile],
    progress: &ProgressFn,
) -> Result<()> {
    let temporary = destination.with_extension("part");
    let total = overrides.len() as u64;
    let result = (|| -> Result<()> {
        let mut writer = zip::ZipWriter::new(File::create(&temporary).at(&temporary)?);
        writer.start_file(manifest_name, zip_options(manifest_name, 0))?;
        std::io::Write::write_all(&mut writer, manifest).at(&temporary)?;
        for (done, file) in overrides.iter().enumerate() {
            progress(ProgressEvent { task: TASK.to_owned(), stage: "Packing files".to_owned(), done: done as u64, total });
            add_file(&mut writer, &format!("overrides/{}", file.path), &file.absolute)?;
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

/// Files for `overrides/`: everything that is not content, plus the content nobody could resolve.
fn override_files<'a>(scan: &'a Scan, unresolved: &[usize]) -> Vec<&'a ExportFile> {
    scan.others
        .iter()
        .chain(unresolved.iter().map(|&index| &scan.candidates[index].file))
        .collect()
}

/// Exports the instance as a `.mrpack`. Files are resolved with Modrinth's `/version_files`.
pub async fn export_mrpack(clients: &Clients, request: &PackExportRequest<'_>, progress: &ProgressFn) -> Result<PackExportReport> {
    let scan = scan(request.game_dir, request.filter, progress)?;
    let hashes: Vec<String> = scan.candidates.iter().map(|candidate| candidate.sha512.clone()).collect();
    let versions = clients.modrinth.versions_by_hashes(&hashes, HashAlgorithm::Sha512).await?;
    let resolved: HashMap<String, Resolved> = versions
        .into_iter()
        .filter_map(|(hash, version)| {
            let file = version.files.iter().find(|file| file.hashes.sha512.as_deref() == Some(hash.as_str()))?;
            Some((
                hash.clone(),
                Resolved { sha1: file.hashes.sha1.clone()?, sha512: hash, url: file.url.clone()?, size: file.size },
            ))
        })
        .collect();

    let (index, unresolved) = build_mrpack_index(request, &scan.candidates, &resolved)?;
    let overrides = override_files(&scan, &unresolved);
    write_pack(request.destination, "modrinth.index.json", &serde_json::to_vec_pretty(&index)?, &overrides, progress)?;
    Ok(PackExportReport { listed: index.files.len(), overrides: overrides.len() })
}

/// Exports the instance as a CurseForge zip. Files are resolved by murmur2 fingerprint.
pub async fn export_curseforge(clients: &Clients, request: &PackExportRequest<'_>, progress: &ProgressFn) -> Result<PackExportReport> {
    let curseforge = clients.require_curseforge()?;
    let scan = scan(request.game_dir, request.filter, progress)?;
    let fingerprints: Vec<u32> = scan.candidates.iter().map(|candidate| candidate.fingerprint).collect();
    let matched = curseforge.versions_by_fingerprints(&fingerprints).await?;
    let mut resolved = HashMap::new();
    for (fingerprint, version) in matched {
        let ids = version.project_id.parse().and_then(|project| version.id.parse().map(|file| (project, file)));
        match ids {
            Ok(ids) => {
                resolved.insert(fingerprint, ids);
            }
            Err(_) => return Err(Error::Other(format!("CurseForge returned non-numeric ids for file {}", version.id))),
        }
    }

    let (manifest, unresolved) = build_curseforge_manifest(request, &scan.candidates, &resolved)?;
    let overrides = override_files(&scan, &unresolved);
    write_pack(request.destination, "manifest.json", &serde_json::to_vec_pretty(&manifest)?, &overrides, progress)?;
    Ok(PackExportReport { listed: manifest.files.len(), overrides: overrides.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::no_progress;

    fn component(uid: &str, version: &str) -> ComponentSpec {
        ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false }
    }

    fn write(game_dir: &Path, relative: &str, content: &str) {
        let path = game_dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn sha512_hex(content: &str) -> String {
        hex::encode(Sha512::digest(content.as_bytes()))
    }

    fn sample_game_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        write(game, "mods/sodium.jar", "sodium");
        write(game, "mods/iris.jar.disabled", "iris");
        write(game, "mods/custom.jar", "my own mod");
        write(game, "mods/.index/sodium.pw.toml", "name = \"Sodium\"\nfilename = \"sodium.jar\"\nside = \"client\"\n");
        write(game, "mods/notes.txt", "not content");
        write(game, "resourcepacks/faithful.zip", "pack");
        write(game, "config/sodium.json", "{}");
        write(game, "logs/latest.log", "log");
        write(game, "options.txt", "fov:90");
        dir
    }

    fn request<'a>(game_dir: &'a Path, components: &'a [ComponentSpec], filter: &'a ExportFilter) -> PackExportRequest<'a> {
        PackExportRequest {
            game_dir,
            destination: Path::new("unused"),
            name: "My pack",
            version: "1.0.0",
            summary: Some("fun"),
            components,
            filter,
        }
    }

    #[test]
    fn scan_separates_identifiable_content_from_overrides_and_drops_indexes() {
        let dir = sample_game_dir();
        let filter = ExportFilter::new(crate::instance::archive::default_excluded());
        let scan = scan(dir.path(), &filter, &no_progress()).unwrap();
        let candidates: Vec<_> = scan.candidates.iter().map(|c| (c.file.path.as_str(), c.disabled, c.side.as_deref())).collect();
        assert_eq!(
            candidates,
            [
                ("mods/custom.jar", false, None),
                ("mods/iris.jar.disabled", true, None),
                ("mods/sodium.jar", false, Some("client")),
                ("resourcepacks/faithful.zip", false, None),
            ]
        );
        let others: Vec<_> = scan.others.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(others, ["config/sodium.json", "mods/notes.txt", "options.txt"]);
    }

    #[test]
    fn mrpack_index_lists_resolved_files_and_sends_the_rest_to_overrides() {
        let dir = sample_game_dir();
        let filter = ExportFilter::new(crate::instance::archive::default_excluded());
        let scan = scan(dir.path(), &filter, &no_progress()).unwrap();
        let components = [component(MINECRAFT_UID, "1.21.4"), component(FABRIC_UID, "0.16.9")];

        // The mocked platform knows sodium and the (disabled) iris, but not the user's own mod or the pack.
        let known = |content: &str, url: &str, size| {
            (
                sha512_hex(content),
                Resolved { sha1: format!("sha1-of-{content}"), sha512: sha512_hex(content), url: url.to_owned(), size },
            )
        };
        let resolved = HashMap::from([
            known("sodium", "https://cdn.modrinth.com/sodium.jar", 6),
            known("iris", "https://cdn.modrinth.com/iris.jar", 4),
        ]);

        let (index, unresolved) = build_mrpack_index(&request(dir.path(), &components, &filter), &scan.candidates, &resolved).unwrap();
        let json = serde_json::to_value(&index).unwrap();
        assert_eq!(json["formatVersion"], 1);
        assert_eq!(json["game"], "minecraft");
        assert_eq!(json["versionId"], "1.0.0");
        assert_eq!(json["summary"], "fun");
        assert_eq!(json["dependencies"], serde_json::json!({"minecraft": "1.21.4", "fabric-loader": "0.16.9"}));

        let files = json["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        let iris = &files[0];
        assert_eq!(iris["path"], "mods/iris.jar", "the .disabled suffix is dropped");
        assert_eq!(iris["env"], serde_json::json!({"client": "optional", "server": "optional"}));
        assert_eq!(iris["downloads"][0], "https://cdn.modrinth.com/iris.jar");
        assert_eq!(iris["hashes"]["sha1"], "sha1-of-iris");
        assert_eq!(iris["fileSize"], 4);
        let sodium = &files[1];
        assert_eq!(sodium["path"], "mods/sodium.jar");
        assert_eq!(sodium["env"], serde_json::json!({"client": "required", "server": "unsupported"}), "client-only per the index");
        assert_eq!(sodium["hashes"]["sha512"], sha512_hex("sodium"));

        let overrides: Vec<_> = override_files(&scan, &unresolved).iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            overrides,
            ["config/sodium.json", "mods/notes.txt", "options.txt", "mods/custom.jar", "resourcepacks/faithful.zip"]
        );
    }

    #[test]
    fn curseforge_manifest_has_the_loader_id_and_required_flags() {
        let dir = sample_game_dir();
        let filter = ExportFilter::new(crate::instance::archive::default_excluded());
        let scan = scan(dir.path(), &filter, &no_progress()).unwrap();
        let components = [component(MINECRAFT_UID, "1.20.1"), component(FORGE_UID, "47.2.0")];
        let fingerprint_of = |path: &str| scan.candidates.iter().find(|c| c.file.path == path).unwrap().fingerprint;
        let resolved = HashMap::from([
            (fingerprint_of("mods/sodium.jar"), (394468, 5_000_001)),
            (fingerprint_of("mods/iris.jar.disabled"), (455508, 5_000_002)),
        ]);

        let (manifest, unresolved) =
            build_curseforge_manifest(&request(dir.path(), &components, &filter), &scan.candidates, &resolved).unwrap();
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["manifestType"], "minecraftModpack");
        assert_eq!(json["manifestVersion"], 1);
        assert_eq!(json["overrides"], "overrides");
        assert_eq!(json["minecraft"]["version"], "1.20.1");
        assert_eq!(json["minecraft"]["modLoaders"], serde_json::json!([{"id": "forge-47.2.0", "primary": true}]));
        assert_eq!(
            json["files"],
            serde_json::json!([
                {"projectID": 455508, "fileID": 5_000_002, "required": false},
                {"projectID": 394468, "fileID": 5_000_001, "required": true}
            ])
        );
        assert_eq!(unresolved.len(), 2);
    }

    #[test]
    fn a_pack_without_a_minecraft_component_cannot_be_exported() {
        let dir = tempfile::tempdir().unwrap();
        let filter = ExportFilter::default();
        assert!(build_mrpack_index(&request(dir.path(), &[], &filter), &[], &HashMap::new()).is_err());
    }

    #[test]
    fn the_archive_holds_the_manifest_and_overrides_that_import_reads_back() {
        let dir = sample_game_dir();
        let filter = ExportFilter::new(crate::instance::archive::default_excluded());
        let scan = scan(dir.path(), &filter, &no_progress()).unwrap();
        let components = [component(MINECRAFT_UID, "1.21.4")];
        let (index, unresolved) = build_mrpack_index(&request(dir.path(), &components, &filter), &scan.candidates, &HashMap::new()).unwrap();
        assert!(index.files.is_empty());

        let out = tempfile::tempdir().unwrap();
        let destination = out.path().join("pack.mrpack");
        let overrides = override_files(&scan, &unresolved);
        write_pack(&destination, "modrinth.index.json", &serde_json::to_vec(&index).unwrap(), &overrides, &no_progress()).unwrap();

        let manifest = crate::content::inspect_modpack(&destination).unwrap();
        assert_eq!(manifest.name, "My pack");
        assert_eq!(manifest.components, [component(MINECRAFT_UID, "1.21.4")]);
        let mut archive = zip::ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        assert!(archive.by_name("overrides/mods/custom.jar").is_ok());
        assert!(archive.by_name("overrides/logs/latest.log").is_err());
        assert!(archive.by_name("overrides/mods/.index/sodium.pw.toml").is_err());
        assert!(!destination.with_extension("part").exists());
    }
}
