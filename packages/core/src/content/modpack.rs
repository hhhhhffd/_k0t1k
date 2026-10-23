//! Modpack import: `.mrpack` (Modrinth) and CurseForge zips. This parses the pack and fills a game
//! dir; creating the instance around it (components, name) is the caller's job.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};

use super::install::checksum_of;
use super::modrinth::HashAlgorithm;
use super::packwiz::{self, PwMod};
use super::{BlockedFile, Clients, Hashes, Platform, Project, ProjectVersion, ResourceKind};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::download::{Checksum, download_checked};
use crate::error::{Error, IoContext, Result};
use crate::instance::exists_in_linked_shared_folder;
use crate::progress::{ProgressEvent, ProgressFn};

const PARALLEL_DOWNLOADS: usize = 16;
const MRPACK_INDEX: &str = "modrinth.index.json";
const CURSEFORGE_MANIFEST: &str = "manifest.json";
const TASK: &str = "modpack";

/// Loader dependency keys (mrpack) and id prefixes (CurseForge) with their Prism meta uids.
const FABRIC_UID: &str = "net.fabricmc.fabric-loader";
const QUILT_UID: &str = "org.quiltmc.quilt-loader";
const FORGE_UID: &str = "net.minecraftforge";
const NEOFORGE_UID: &str = "net.neoforged";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModpackFormat {
    Modrinth,
    CurseForge,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackManifest {
    pub format: ModpackFormat,
    pub name: String,
    pub version: String,
    pub summary: Option<String>,
    /// `net.minecraft` first, then the loader if the pack has one.
    pub components: Vec<ComponentSpec>,
    pub recommended_ram_mb: Option<u32>,
    pub source: ModpackSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModpackSource {
    Modrinth {
        files: Vec<MrpackFile>,
    },
    CurseForge {
        files: Vec<CurseForgePackFile>,
        overrides: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: Hashes,
    #[serde(default)]
    pub downloads: Vec<String>,
    pub file_size: u64,
    pub env: Option<MrpackEnv>,
}

/// `required`, `optional` or `unsupported` per side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackEnv {
    pub client: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurseForgePackFile {
    #[serde(rename = "projectID")]
    pub project_id: u32,
    #[serde(rename = "fileID")]
    pub file_id: u32,
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackInstallReport {
    /// Files the author forbids third parties to download; the user has to supply them.
    pub blocked: Vec<BlockedFile>,
    /// Installed files the pack marks as optional (paths relative to the game dir); all are enabled.
    pub optional_files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackIndex {
    format_version: u32,
    game: String,
    version_id: String,
    name: String,
    summary: Option<String>,
    files: Vec<MrpackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeManifest {
    manifest_type: String,
    manifest_version: u32,
    #[serde(default = "default_pack_name")]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default = "default_overrides")]
    overrides: String,
    minecraft: CurseForgeMinecraft,
    files: Vec<CurseForgePackFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CurseForgeLoader>,
    recommended_ram: Option<u32>,
}

#[derive(Deserialize)]
struct CurseForgeLoader {
    id: String,
}

fn default_pack_name() -> String {
    "Unnamed".to_owned()
}

fn default_overrides() -> String {
    "overrides".to_owned()
}

/// Joins an archive-supplied relative path onto `root`, rejecting absolute paths, drive prefixes,
/// `.`/`..` and empty segments, so nothing can escape `root` (zip-slip).
pub(super) fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let normalized = relative.replace('\\', "/");
    let mut path = root.to_path_buf();
    for segment in normalized.split('/') {
        let is_plain = Path::new(segment).components().count() == 1
            && matches!(
                Path::new(segment).components().next(),
                Some(Component::Normal(_))
            )
            && !segment.contains(':');
        if !is_plain {
            return Err(Error::Other(format!(
                "unsafe path in modpack: {relative:?}"
            )));
        }
        path.push(segment);
    }
    Ok(path)
}

/// Reads the pack description from a `.mrpack` or CurseForge zip.
pub fn inspect_modpack(zip_path: &Path) -> Result<ModpackManifest> {
    let mut archive = zip::ZipArchive::new(File::open(zip_path).at(zip_path)?)?;
    // Modrinth first, so an `overrides/manifest.json` cannot be mistaken for the CurseForge manifest.
    if archive.index_for_name(MRPACK_INDEX).is_some() {
        let index: MrpackIndex = serde_json::from_reader(archive.by_name(MRPACK_INDEX)?)?;
        parse_mrpack(index)
    } else if archive.index_for_name(CURSEFORGE_MANIFEST).is_some() {
        let manifest: CurseForgeManifest =
            serde_json::from_reader(archive.by_name(CURSEFORGE_MANIFEST)?)?;
        parse_curseforge(manifest)
    } else {
        Err(Error::Other(
            "Archive does not contain a recognized modpack type.".into(),
        ))
    }
}

fn parse_mrpack(index: MrpackIndex) -> Result<ModpackManifest> {
    if index.format_version != 1 || index.game != "minecraft" {
        return Err(Error::Other(format!(
            "unsupported mrpack: formatVersion {} for game {:?}",
            index.format_version, index.game
        )));
    }
    let minecraft = index
        .dependencies
        .get("minecraft")
        .ok_or_else(|| Error::Other("the mrpack does not declare a Minecraft version".into()))?;
    let mut components = vec![component(MINECRAFT_UID, minecraft)];
    for (key, version) in &index.dependencies {
        let uid = match key.as_str() {
            "minecraft" => continue,
            "fabric-loader" => FABRIC_UID,
            "quilt-loader" => QUILT_UID,
            "forge" => FORGE_UID,
            "neoforge" => NEOFORGE_UID,
            other => {
                tracing::warn!("ignoring unknown mrpack dependency {other:?}");
                continue;
            }
        };
        components.push(component(uid, version));
    }
    Ok(ModpackManifest {
        format: ModpackFormat::Modrinth,
        name: index.name,
        version: index.version_id,
        summary: index.summary,
        components,
        recommended_ram_mb: None,
        source: ModpackSource::Modrinth { files: index.files },
    })
}

fn parse_curseforge(manifest: CurseForgeManifest) -> Result<ModpackManifest> {
    if manifest.manifest_type != "minecraftModpack" || manifest.manifest_version != 1 {
        return Err(Error::Other(format!(
            "unsupported CurseForge manifest: {} version {}",
            manifest.manifest_type, manifest.manifest_version
        )));
    }
    let minecraft = manifest.minecraft.version.trim_end_matches('.');
    let mut components = vec![component(MINECRAFT_UID, minecraft)];

    // Several loaders may be listed; the last recognised one wins, as in Prism.
    let mut loader = None;
    for entry in &manifest.minecraft.mod_loaders {
        let Some((prefix, version)) = entry.id.split_once('-') else {
            tracing::warn!("ignoring malformed CurseForge loader id {:?}", entry.id);
            continue;
        };
        let uid = match prefix {
            "neoforge" => NEOFORGE_UID,
            "forge" => FORGE_UID,
            "fabric" => FABRIC_UID,
            "quilt" => QUILT_UID,
            _ => {
                tracing::warn!("ignoring unknown CurseForge loader {:?}", entry.id);
                continue;
            }
        };
        // Old NeoForge ids repeat the Minecraft version: `neoforge-1.20.1-47.1.106`.
        let version = version
            .strip_prefix(&format!("{minecraft}-"))
            .unwrap_or(version);
        if version == "recommended" {
            return Err(Error::Other(format!(
                "loader {:?} has no concrete version",
                entry.id
            )));
        }
        loader = Some(component(uid, version));
    }
    components.extend(loader);

    Ok(ModpackManifest {
        format: ModpackFormat::CurseForge,
        name: manifest.name,
        version: manifest.version,
        summary: None,
        components,
        recommended_ram_mb: manifest.minecraft.recommended_ram.filter(|ram| *ram > 0),
        source: ModpackSource::CurseForge {
            files: manifest.files,
            overrides: manifest.overrides,
        },
    })
}

fn component(uid: &str, version: &str) -> ComponentSpec {
    ComponentSpec {
        uid: uid.to_owned(),
        version: version.to_owned(),
        dependency_only: false,
    }
}

/// A file to fetch into the game dir.
struct Download {
    relative_path: String,
    destination: PathBuf,
    /// Tried in order; the first one that downloads and verifies wins.
    urls: Vec<String>,
    checksum: Option<Checksum>,
}

/// Downloads `manifest`'s files into `game_dir`, then unpacks the pack's override folders over them.
/// Metadata (`.pw.toml`) is written for mods whose project could be identified.
pub async fn install_modpack(
    manifest: &ModpackManifest,
    zip_path: &Path,
    game_dir: &Path,
    clients: &Clients,
    progress: &ProgressFn,
) -> Result<ModpackInstallReport> {
    match &manifest.source {
        ModpackSource::Modrinth { files } => {
            install_mrpack(files, zip_path, game_dir, clients, progress).await
        }
        ModpackSource::CurseForge { files, overrides } => {
            install_curseforge(files, overrides, zip_path, game_dir, clients, progress).await
        }
    }
}

async fn install_mrpack(
    files: &[MrpackFile],
    zip_path: &Path,
    game_dir: &Path,
    clients: &Clients,
    progress: &ProgressFn,
) -> Result<ModpackInstallReport> {
    let mut report = ModpackInstallReport::default();
    let mut downloads = Vec::new();
    // Every path is validated before anything is downloaded, so a hostile pack writes nothing.
    for file in files {
        let client_env = file.env.as_ref().and_then(|env| env.client.as_deref());
        if client_env == Some("unsupported") {
            continue;
        }
        let destination = safe_join(game_dir, &file.path)?;
        let sha512 = file
            .hashes
            .sha512
            .clone()
            .ok_or_else(|| Error::Other(format!("{} has no sha512 hash", file.path)))?;
        if file.downloads.is_empty() {
            return Err(Error::Other(format!("{} has no download URLs", file.path)));
        }
        if client_env == Some("optional") {
            report.optional_files.push(file.path.replace('\\', "/"));
        }
        downloads.push(Download {
            relative_path: file.path.replace('\\', "/"),
            destination,
            urls: file.downloads.clone(),
            checksum: Some(Checksum::Sha512(sha512.to_ascii_lowercase())),
        });
    }

    run_downloads(clients, &downloads, game_dir, progress).await?;
    extract_overrides(
        zip_path,
        game_dir,
        &["overrides", "client-overrides"],
        progress,
    )
    .await?;

    let mod_hashes: Vec<String> = downloads
        .iter()
        .filter(|download| is_direct_mod(&download.relative_path))
        .filter_map(|download| match &download.checksum {
            Some(Checksum::Sha512(hash)) => Some(hash.clone()),
            _ => None,
        })
        .collect();
    let versions = clients
        .modrinth
        .versions_by_hashes(&mod_hashes, HashAlgorithm::Sha512)
        .await?;
    let mut project_ids: Vec<String> = versions
        .values()
        .map(|version| version.project_id.clone())
        .collect();
    project_ids.sort();
    project_ids.dedup();
    let projects: HashMap<String, Project> = clients
        .modrinth
        .projects(&project_ids)
        .await?
        .into_iter()
        .map(|project| (project.id.clone(), project))
        .collect();
    let index_dir = packwiz::index_dir(game_dir, ResourceKind::Mod);
    for (hash, version) in &versions {
        let (Some(project), Some(file)) = (
            projects.get(&version.project_id),
            version
                .files
                .iter()
                .find(|file| file.hashes.sha512.as_deref() == Some(hash.as_str())),
        ) else {
            continue;
        };
        packwiz::write(
            &index_dir,
            &project.slug,
            &PwMod::new(project, version, file, true)?,
        )?;
    }
    Ok(report)
}

/// Prism only indexes mods that sit directly in `mods/`.
fn is_direct_mod(relative_path: &str) -> bool {
    relative_path
        .strip_prefix("mods/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
}

async fn install_curseforge(
    files: &[CurseForgePackFile],
    overrides: &str,
    zip_path: &Path,
    game_dir: &Path,
    clients: &Clients,
    progress: &ProgressFn,
) -> Result<ModpackInstallReport> {
    safe_join(game_dir, overrides)?;
    let curseforge = clients.require_curseforge()?;
    let mut report = ModpackInstallReport::default();

    let file_ids: Vec<String> = files.iter().map(|file| file.file_id.to_string()).collect();
    let versions = curseforge.versions_by_ids(&file_ids).await?;
    let by_file: HashMap<&str, &ProjectVersion> = versions
        .iter()
        .map(|version| (version.id.as_str(), version))
        .collect();
    let missing: Vec<String> = file_ids
        .iter()
        .filter(|id| !by_file.contains_key(id.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(Error::Other(format!(
            "CurseForge does not know the files {}",
            missing.join(", ")
        )));
    }

    // Files with downloads disabled are often also on Modrinth; the same SHA-1 means the same file.
    let blocked_hashes: Vec<String> = versions
        .iter()
        .filter_map(|version| version.install_file())
        .filter(|file| file.url.is_none())
        .filter_map(|file| file.hashes.sha1.clone())
        .collect();
    let modrinth_copies = clients
        .modrinth
        .versions_by_hashes(&blocked_hashes, HashAlgorithm::Sha1)
        .await?;

    let mut project_ids: Vec<String> = versions
        .iter()
        .map(|version| version.project_id.clone())
        .collect();
    project_ids.sort();
    project_ids.dedup();
    let projects: HashMap<String, Project> = curseforge
        .projects(&project_ids)
        .await?
        .into_iter()
        .map(|project| (project.id.clone(), project))
        .collect();

    let mut downloads = Vec::new();
    let mut mod_metadata = Vec::new();
    for pack_file in files {
        let version = by_file[pack_file.file_id.to_string().as_str()];
        let file = version.install_file().ok_or_else(|| {
            Error::Other(format!(
                "CurseForge file {} has no download information",
                version.id
            ))
        })?;
        let project = projects.get(&version.project_id).ok_or_else(|| {
            Error::Other(format!(
                "CurseForge project {} not found",
                version.project_id
            ))
        })?;
        let kind = project.kind.unwrap_or(ResourceKind::Mod);
        let file_name: String = file
            .filename
            .chars()
            .filter(|c| !c.is_control() && !"\\/:*?\"<>|".contains(*c))
            .collect();
        let relative_path = format!("{}/{file_name}", kind.folder());
        let destination = safe_join(game_dir, &relative_path)?;

        let url = file.url.clone().or_else(|| {
            let sha1 = file.hashes.sha1.as_ref()?;
            modrinth_copies
                .get(sha1)?
                .files
                .iter()
                .find(|f| f.hashes.sha1.as_ref() == Some(sha1))?
                .url
                .clone()
        });
        let Some(url) = url else {
            report.blocked.push(BlockedFile {
                platform: Platform::CurseForge,
                project_id: version.project_id.clone(),
                name: project.title.clone(),
                file_name,
                target_path: relative_path,
                download_page: file.download_page.clone(),
                sha1: file.hashes.sha1.clone(),
            });
            continue;
        };
        if !pack_file.required {
            report.optional_files.push(relative_path.clone());
        }
        if kind == ResourceKind::Mod {
            mod_metadata.push((
                project.slug.clone(),
                PwMod::new(project, version, file, true)?,
            ));
        }
        downloads.push(Download {
            relative_path,
            destination,
            urls: vec![url],
            checksum: checksum_of(file),
        });
    }

    run_downloads(clients, &downloads, game_dir, progress).await?;
    extract_overrides(zip_path, game_dir, &[overrides], progress).await?;

    let index_dir = packwiz::index_dir(game_dir, ResourceKind::Mod);
    for (slug, pw) in mod_metadata {
        packwiz::write(&index_dir, &slug, &pw)?;
    }
    Ok(report)
}

/// Files that already exist in a linked shared folder are kept: they are the user's, shared with
/// other instances, and a pack from the internet must not replace them.
async fn run_downloads(
    clients: &Clients,
    downloads: &[Download],
    game_dir: &Path,
    progress: &ProgressFn,
) -> Result<()> {
    let downloads: Vec<&Download> = downloads
        .iter()
        .filter(|download| !exists_in_linked_shared_folder(game_dir, &download.destination))
        .collect();
    let total = downloads.len() as u64;
    let done = Arc::new(AtomicU64::new(0));
    let report = |done: u64| {
        progress(ProgressEvent {
            task: TASK.to_owned(),
            stage: "Downloading files".to_owned(),
            done,
            total,
        })
    };
    report(0);

    stream::iter(downloads.iter().copied())
        .map(|download| {
            let done = done.clone();
            async move {
                let mut last_error = None;
                for url in &download.urls {
                    match download_checked(
                        &clients.http,
                        url,
                        &download.destination,
                        download.checksum.as_ref(),
                    )
                    .await
                    {
                        Ok(()) => {
                            last_error = None;
                            break;
                        }
                        Err(error) => {
                            tracing::warn!(
                                "download of {} from {url} failed: {error}",
                                download.relative_path
                            );
                            last_error = Some(error);
                        }
                    }
                }
                if let Some(error) = last_error {
                    return Err(error);
                }
                report(done.fetch_add(1, Ordering::Relaxed) + 1);
                Ok(())
            }
        })
        .buffer_unordered(PARALLEL_DOWNLOADS)
        .try_collect::<()>()
        .await
}

/// Unpacks `<folder>/...` entries for each folder in order, later folders overwriting earlier ones.
/// Files that already exist in a linked shared folder are kept (see `run_downloads`).
async fn extract_overrides(
    zip_path: &Path,
    game_dir: &Path,
    folders: &[&str],
    progress: &ProgressFn,
) -> Result<()> {
    progress(ProgressEvent {
        task: TASK.to_owned(),
        stage: "Extracting overrides".to_owned(),
        done: 0,
        total: 1,
    });
    let (zip_path, game_dir) = (zip_path.to_owned(), game_dir.to_owned());
    let folders: Vec<String> = folders.iter().map(|folder| (*folder).to_owned()).collect();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut archive = zip::ZipArchive::new(File::open(&zip_path).at(&zip_path)?)?;
        for folder in &folders {
            let prefix = format!("{}/", folder.trim_end_matches('/'));
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index)?;
                let Some(relative) = entry.name()?.strip_prefix(&prefix).map(str::to_owned) else {
                    continue;
                };
                let relative = relative.trim_end_matches('/');
                if relative.is_empty() {
                    continue;
                }
                let destination = safe_join(&game_dir, relative)?;
                if entry.is_dir() {
                    std::fs::create_dir_all(&destination).at(&destination)?;
                    continue;
                }
                if exists_in_linked_shared_folder(&game_dir, &destination) {
                    continue;
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).at(parent)?;
                }
                let mut file = File::create(&destination).at(&destination)?;
                std::io::copy(&mut entry, &mut file).at(&destination)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| Error::Other(format!("override extraction task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::progress::no_progress;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("k0t1k-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("pack.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, content) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    const MRPACK: &str = r#"{"formatVersion":1,"game":"minecraft","versionId":"1.2.0","name":"Fabulously","summary":"s",
        "files":[
          {"path":"mods\\sodium.jar","hashes":{"sha1":"a","sha512":"b"},"downloads":["https://cdn.modrinth.com/x.jar"],"fileSize":3,"env":{"client":"required","server":"required"}},
          {"path":"mods/server-only.jar","hashes":{"sha1":"a","sha512":"c"},"downloads":["https://cdn.modrinth.com/y.jar"],"fileSize":3,"env":{"client":"unsupported","server":"required"}}],
        "dependencies":{"minecraft":"1.21.4","fabric-loader":"0.16.9","something-new":"1"}}"#;

    const CURSEFORGE: &str = r#"{"minecraft":{"version":"1.20.1.","modLoaders":[{"id":"forge-47.2.0","primary":false},
        {"id":"neoforge-1.20.1-47.1.106","primary":true},{"id":"liteloader-1","primary":false}],"recommendedRam":4096},
        "manifestType":"minecraftModpack","manifestVersion":1,"name":"Pack","version":"2","author":"me","overrides":"overrides",
        "files":[{"projectID":1,"fileID":2,"required":true},{"projectID":3,"fileID":4}]}"#;

    #[test]
    fn safe_join_rejects_escapes() {
        let root = Path::new("/game");
        assert_eq!(
            safe_join(root, "mods\\a.jar").unwrap(),
            Path::new("/game/mods/a.jar")
        );
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "./a",
            "C:/x",
            "C:\\x",
            "\\\\server\\share",
            "a/",
        ] {
            assert!(safe_join(root, bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn parses_mrpack_index() {
        let manifest = parse_mrpack(serde_json::from_str(MRPACK).unwrap()).unwrap();
        assert_eq!(manifest.name, "Fabulously");
        assert_eq!(manifest.version, "1.2.0");
        let uids: Vec<_> = manifest
            .components
            .iter()
            .map(|c| (c.uid.as_str(), c.version.as_str()))
            .collect();
        assert_eq!(
            uids,
            [
                ("net.minecraft", "1.21.4"),
                ("net.fabricmc.fabric-loader", "0.16.9")
            ]
        );
    }

    #[test]
    fn parses_curseforge_manifest_last_loader_wins() {
        let manifest = parse_curseforge(serde_json::from_str(CURSEFORGE).unwrap()).unwrap();
        let uids: Vec<_> = manifest
            .components
            .iter()
            .map(|c| (c.uid.as_str(), c.version.as_str()))
            .collect();
        assert_eq!(
            uids,
            [("net.minecraft", "1.20.1"), ("net.neoforged", "47.1.106")]
        );
        assert_eq!(manifest.recommended_ram_mb, Some(4096));
        let ModpackSource::CurseForge { files, .. } = manifest.source else {
            panic!("wrong source");
        };
        assert!(files[1].required, "required defaults to true");
    }

    #[test]
    fn rejects_wrong_format_version() {
        let mut index: MrpackIndex = serde_json::from_str(MRPACK).unwrap();
        index.format_version = 2;
        assert!(parse_mrpack(index).is_err());
    }

    #[test]
    fn inspect_prefers_modrinth_over_nested_manifest() {
        let dir = temp_dir("inspect");
        let zip = write_zip(
            &dir,
            &[("overrides/manifest.json", "{}"), (MRPACK_INDEX, MRPACK)],
        );
        assert_eq!(
            inspect_modpack(&zip).unwrap().format,
            ModpackFormat::Modrinth
        );

        let zip = write_zip(&dir, &[(CURSEFORGE_MANIFEST, CURSEFORGE)]);
        assert_eq!(
            inspect_modpack(&zip).unwrap().format,
            ModpackFormat::CurseForge
        );

        let zip = write_zip(&dir, &[("readme.txt", "x")]);
        assert!(inspect_modpack(&zip).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    fn link_shared_folder(game_dir: &Path, shared: &Path, folder: &str) {
        std::fs::create_dir_all(shared).unwrap();
        std::fs::create_dir_all(game_dir).unwrap();
        std::os::unix::fs::symlink(shared, game_dir.join(folder)).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn overrides_do_not_replace_files_in_shared_folders() {
        let dir = temp_dir("shared-packs");
        let game_dir = dir.join("game");
        let (packs, shaders) = (dir.join("shared-r"), dir.join("shared-s"));
        link_shared_folder(&game_dir, &packs, "resourcepacks");
        link_shared_folder(&game_dir, &shaders, "shaderpacks");
        std::fs::write(packs.join("mine.zip"), "user").unwrap();
        std::fs::write(shaders.join("mine.zip"), "user").unwrap();
        // Not linked: overwritten as usual.
        std::fs::create_dir_all(game_dir.join("screenshots")).unwrap();
        std::fs::write(game_dir.join("screenshots/mine.png"), "user").unwrap();

        let zip = write_zip(
            &dir,
            &[
                ("overrides/resourcepacks/mine.zip", "pack"),
                ("overrides/resourcepacks/new.zip", "pack"),
                ("overrides/shaderpacks/mine.zip", "pack"),
                ("overrides/shaderpacks/new.zip", "pack"),
                ("overrides/screenshots/mine.png", "pack"),
                ("overrides/config/a.txt", "pack"),
            ],
        );
        extract_overrides(&zip, &game_dir, &["overrides"], &no_progress())
            .await
            .unwrap();
        for shared in [&packs, &shaders] {
            assert_eq!(std::fs::read_to_string(shared.join("mine.zip")).unwrap(), "user");
            assert_eq!(std::fs::read_to_string(shared.join("new.zip")).unwrap(), "pack");
        }
        assert_eq!(std::fs::read_to_string(game_dir.join("screenshots/mine.png")).unwrap(), "pack");
        assert_eq!(std::fs::read_to_string(game_dir.join("config/a.txt")).unwrap(), "pack");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn downloads_do_not_replace_files_in_shared_folders() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let dir = temp_dir("shared-downloads");
        let game_dir = dir.join("game");
        let shaders = dir.join("shared-s");
        link_shared_folder(&game_dir, &shaders, "shaderpacks");
        std::fs::write(shaders.join("mine.zip"), "user").unwrap();

        // The URL is unreachable and the checksum wrong: any attempt to download would fail.
        let download = Download {
            relative_path: "shaderpacks/mine.zip".to_owned(),
            destination: game_dir.join("shaderpacks/mine.zip"),
            urls: vec!["http://127.0.0.1:1/mine.zip".to_owned()],
            checksum: Some(Checksum::Sha512("0".repeat(128))),
        };
        run_downloads(&clients, &[download], &game_dir, &no_progress()).await.unwrap();
        assert_eq!(std::fs::read_to_string(shaders.join("mine.zip")).unwrap(), "user");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn client_overrides_win_and_zip_slip_is_rejected() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let empty = r#"{"formatVersion":1,"game":"minecraft","versionId":"1","name":"n","files":[],"dependencies":{"minecraft":"1.21.4"}}"#;

        let dir = temp_dir("overrides");
        let zip = write_zip(
            &dir,
            &[
                (MRPACK_INDEX, empty),
                ("overrides/config/a.txt", "common"),
                ("overrides/config/b.txt", "common"),
                ("client-overrides/config/a.txt", "client"),
            ],
        );
        let game_dir = dir.join("game");
        let manifest = inspect_modpack(&zip).unwrap();
        install_modpack(&manifest, &zip, &game_dir, &clients, &no_progress())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(game_dir.join("config/a.txt")).unwrap(),
            "client"
        );
        assert_eq!(
            std::fs::read_to_string(game_dir.join("config/b.txt")).unwrap(),
            "common"
        );

        let evil = dir.join("evil");
        std::fs::create_dir_all(&evil).unwrap();
        let zip = write_zip(
            &evil,
            &[(MRPACK_INDEX, empty), ("overrides/../escaped.txt", "x")],
        );
        let manifest = inspect_modpack(&zip).unwrap();
        let outcome = install_modpack(
            &manifest,
            &zip,
            &evil.join("game"),
            &clients,
            &no_progress(),
        )
        .await;
        assert!(outcome.is_err());
        assert!(!evil.join("escaped.txt").exists() && !evil.join("game/escaped.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn mrpack_with_unsafe_file_path_downloads_nothing() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let index = MRPACK.replace("mods\\\\sodium.jar", "..\\\\evil.jar");
        let manifest = parse_mrpack(serde_json::from_str(&index).unwrap()).unwrap();
        let dir = temp_dir("unsafe");
        let zip = write_zip(&dir, &[(MRPACK_INDEX, &index)]);
        assert!(
            install_modpack(&manifest, &zip, &dir.join("game"), &clients, &no_progress())
                .await
                .is_err()
        );
        assert!(!dir.join("game").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
