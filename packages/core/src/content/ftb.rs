//! Feed The Beast modpacks: the public `api.feed-the-beast.com` API Prism's FTB page uses (the former
//! modpacks.ch). A version manifest lists every file with its URLs and SHA-1; files hosted on
//! CurseForge also carry their CurseForge ids.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};

use super::{BlockedFile, Clients, Loader, Platform, safe_join, send_json};
use crate::download::{Checksum, download_checked};
use crate::error::{Error, Result};
use crate::instance::exists_in_linked_shared_folder;
use crate::progress::{ProgressEvent, ProgressFn};

const BASE_URL: &str = "https://api.feed-the-beast.com/v1/modpacks/public";
const MAX_RESULTS: usize = 30;
const PARALLEL_PACK_DETAILS: usize = 8;
const PARALLEL_DOWNLOADS: usize = 16;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FtbPack {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub authors: Vec<String>,
    pub icon_url: Option<String>,
    pub installs: u64,
    /// RFC 3339.
    pub updated: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FtbLoaderRef {
    pub id: Loader,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FtbVersion {
    pub id: u64,
    pub name: String,
    /// `release`, `beta` or `alpha`.
    #[serde(rename = "type")]
    pub kind: String,
    /// RFC 3339.
    pub updated: String,
    pub minecraft_version: String,
    pub loader: Option<FtbLoaderRef>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FtbCurseForgeRef {
    pub project: String,
    pub file: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FtbFile {
    /// Folder relative to the game dir, e.g. `./mods`.
    pub path: String,
    pub name: String,
    pub url: String,
    pub mirrors: Vec<String>,
    pub sha1: String,
    pub clientonly: bool,
    pub serveronly: bool,
    pub curseforge: Option<FtbCurseForgeRef>,
}

#[derive(Debug, Clone)]
pub struct FtbManifest {
    pub minecraft_version: String,
    pub loader: Option<FtbLoaderRef>,
    pub files: Vec<FtbFile>,
}

#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    /// `http` should come from `download::http_client()` so requests carry our User-Agent.
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, base_url: BASE_URL.to_owned() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// The most installed packs for an empty query, else the packs matching `query`.
    pub async fn search(&self, query: &str) -> Result<Vec<FtbPack>> {
        let query = query.trim();
        let request = if query.is_empty() {
            self.http.get(self.url(&format!("/modpack/popular/installs/{MAX_RESULTS}")))
        } else {
            self.http.get(self.url(&format!("/modpack/search/{MAX_RESULTS}"))).query(&[("term", query)])
        };
        let ids: RawIds = send_json(request).await?;
        ids.check()?;
        // The list endpoints return only ids; details are fetched concurrently and keep the list order.
        stream::iter(ids.packs.into_iter().take(MAX_RESULTS))
            .map(|id| async move { (id, self.raw_pack(id).await) })
            .buffered(PARALLEL_PACK_DETAILS)
            .filter_map(|(id, pack)| async move {
                match pack {
                    Ok(pack) => Some(Ok(pack.into_pack())),
                    // One broken pack must not hide the rest of the list.
                    Err(error) => {
                        tracing::warn!("FTB pack {id} was skipped: {error}");
                        None
                    }
                }
            })
            .try_collect()
            .await
    }

    /// Versions of a pack, newest first. Private versions and versions without a Minecraft target are left out.
    pub async fn versions(&self, pack_id: u64) -> Result<Vec<FtbVersion>> {
        Ok(self.raw_pack(pack_id).await?.into_versions())
    }

    /// Name and logo of a pack.
    pub async fn pack(&self, pack_id: u64) -> Result<FtbPack> {
        Ok(self.raw_pack(pack_id).await?.into_pack())
    }

    pub async fn manifest(&self, pack_id: u64, version_id: u64) -> Result<FtbManifest> {
        let raw: RawManifest = send_json(self.http.get(self.url(&format!("/modpack/{pack_id}/{version_id}")))).await?;
        raw.status.check(&raw.message)?;
        let (minecraft_version, loader) = targets_of(&raw.targets);
        let minecraft_version =
            minecraft_version.ok_or_else(|| Error::Other(format!("FTB pack {pack_id} version {version_id} has no Minecraft target")))?;
        Ok(FtbManifest { minecraft_version, loader, files: raw.files })
    }

    async fn raw_pack(&self, pack_id: u64) -> Result<RawPack> {
        let raw: RawPack = send_json(self.http.get(self.url(&format!("/modpack/{pack_id}")))).await?;
        raw.status.check(&raw.message)?;
        Ok(raw)
    }
}

/// Downloads the client-side files of a pack into `game_dir`, verifying SHA-1. Files that cannot be
/// downloaded because CurseForge forbids third parties to are returned for the user to supply.
pub async fn install_files(clients: &Clients, game_dir: &Path, files: &[FtbFile], task: &str, progress: &ProgressFn) -> Result<Vec<BlockedFile>> {
    let files: Vec<&FtbFile> = files.iter().filter(|file| !file.serveronly).collect();
    let total = files.len() as u64;
    let done = Arc::new(AtomicU64::new(0));
    let report = |done: u64| {
        progress(ProgressEvent { task: task.to_owned(), stage: "Downloading files".to_owned(), done, total });
    };
    report(0);

    let blocked: Vec<Option<BlockedFile>> = stream::iter(files)
        .map(|file| {
            let done = done.clone();
            let report = &report;
            async move {
                let blocked = install_file(clients, game_dir, file).await?;
                report(done.fetch_add(1, Ordering::Relaxed) + 1);
                Ok::<_, Error>(blocked)
            }
        })
        .buffer_unordered(PARALLEL_DOWNLOADS)
        .try_collect()
        .await?;
    Ok(blocked.into_iter().flatten().collect())
}

/// Path of a file relative to the game dir, with `/` separators and no `.` segments.
fn relative_path(file: &FtbFile) -> String {
    let mut segments: Vec<&str> = file.path.split(['/', '\\']).filter(|segment| !segment.is_empty() && *segment != ".").collect();
    segments.push(&file.name);
    segments.join("/")
}

async fn install_file(clients: &Clients, game_dir: &Path, file: &FtbFile) -> Result<Option<BlockedFile>> {
    let relative = relative_path(file);
    let destination = safe_join(game_dir, &relative)?;
    // A file in a linked shared folder is the user's, shared with other instances (see modpack install).
    if exists_in_linked_shared_folder(game_dir, &destination) {
        return Ok(None);
    }
    let checksum = (!file.sha1.is_empty()).then(|| Checksum::Sha1(file.sha1.to_ascii_lowercase()));

    let mut urls: Vec<String> = std::iter::once(&file.url).chain(&file.mirrors).filter(|url| !url.is_empty()).cloned().collect();
    let mut download_page = None;
    if urls.is_empty() {
        let Some(curseforge) = &file.curseforge else {
            return Err(Error::Other(format!("FTB file {relative} has no download URL")));
        };
        // Resolved only when a CurseForge key is configured; otherwise the user gets the file as blocked.
        if let Some(client) = &clients.curseforge {
            let version = client.version(&curseforge.project, &curseforge.file).await?;
            if let Some(found) = version.install_file() {
                urls.extend(found.url.clone());
                download_page = found.download_page.clone();
            }
        }
        if urls.is_empty() {
            return Ok(Some(BlockedFile {
                platform: Platform::CurseForge,
                project_id: curseforge.project.clone(),
                name: file.name.clone(),
                file_name: file.name.clone(),
                target_path: relative,
                download_page,
                sha1: checksum.is_some().then(|| file.sha1.clone()),
            }));
        }
    }

    let mut last_error = None;
    for url in &urls {
        match download_checked(&clients.http, url, &destination, checksum.as_ref()).await {
            Ok(()) => return Ok(None),
            Err(error) => {
                tracing::warn!("download of {relative} from {url} failed: {error}");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("urls is not empty"))
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Status {
    #[default]
    Success,
    Error,
}

impl Status {
    fn check(&self, message: &str) -> Result<()> {
        match self {
            Status::Success => Ok(()),
            Status::Error => Err(Error::Other(format!("FTB API error: {message}"))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawIds {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    packs: Vec<u64>,
}

impl RawIds {
    fn check(&self) -> Result<()> {
        self.status.check(&self.message)
    }
}

#[derive(Debug, Deserialize)]
struct RawPack {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    id: u64,
    name: String,
    #[serde(default)]
    synopsis: String,
    #[serde(default)]
    updated: i64,
    #[serde(default)]
    installs: u64,
    #[serde(default)]
    tags: Vec<RawNamed>,
    #[serde(default)]
    art: Vec<RawArt>,
    #[serde(default)]
    authors: Vec<RawNamed>,
    #[serde(default)]
    versions: Vec<RawVersion>,
}

#[derive(Debug, Deserialize)]
struct RawNamed {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawArt {
    url: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RawVersion {
    id: u64,
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    updated: i64,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    targets: Vec<RawTarget>,
}

#[derive(Debug, Deserialize)]
struct RawTarget {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct RawManifest {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    targets: Vec<RawTarget>,
    #[serde(default)]
    files: Vec<FtbFile>,
}

fn iso(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0).unwrap_or_default().to_rfc3339()
}

/// Minecraft version and loader out of a version's `targets` (`java` and others are ignored).
fn targets_of(targets: &[RawTarget]) -> (Option<String>, Option<FtbLoaderRef>) {
    let minecraft = targets.iter().find(|target| target.name == "minecraft").map(|target| target.version.clone());
    let loader = targets.iter().find_map(|target| {
        let id = match target.name.as_str() {
            "fabric" => Loader::Fabric,
            "quilt" => Loader::Quilt,
            "forge" => Loader::Forge,
            "neoforge" => Loader::NeoForge,
            _ => return None,
        };
        Some(FtbLoaderRef { id, version: target.version.clone() })
    });
    (minecraft, loader)
}

impl RawPack {
    fn into_pack(self) -> FtbPack {
        // The square logo is what Prism shows; any art is better than none.
        let icon_url = self.art.iter().find(|art| art.kind == "square").or(self.art.first()).map(|art| art.url.clone());
        FtbPack {
            id: self.id,
            name: self.name,
            summary: self.synopsis,
            authors: self.authors.into_iter().map(|author| author.name).collect(),
            icon_url,
            installs: self.installs,
            updated: iso(self.updated),
            tags: self.tags.into_iter().map(|tag| tag.name).collect(),
        }
    }

    fn into_versions(self) -> Vec<FtbVersion> {
        let mut versions: Vec<(i64, FtbVersion)> = self
            .versions
            .into_iter()
            .filter(|version| !version.private)
            .filter_map(|version| {
                let (minecraft, loader) = targets_of(&version.targets);
                Some((
                    version.updated,
                    FtbVersion {
                        id: version.id,
                        name: version.name,
                        kind: version.kind,
                        updated: iso(version.updated),
                        minecraft_version: minecraft?,
                        loader,
                    },
                ))
            })
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.id.cmp(&a.1.id)));
        versions.into_iter().map(|(_, version)| version).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK: &str = r#"{"status":"success","id":135,"name":"FTB Skies","synopsis":"Sky pack",
      "updated":1700000000,"installs":42,"tags":[{"id":1,"name":"Tech"}],
      "art":[{"url":"https://x/splash.png","type":"splash"},{"url":"https://x/logo.webp","type":"square"}],
      "authors":[{"id":-1,"name":"FTB Team"}],
      "versions":[
        {"id":1,"name":"1.0","type":"release","updated":1600000000,"targets":[
          {"name":"minecraft","version":"1.20.1","type":"game"},{"name":"neoforge","version":"47.1","type":"modloader"},{"name":"java","version":"17","type":"runtime"}]},
        {"id":2,"name":"1.1","type":"beta","updated":1650000000,"targets":[{"name":"minecraft","version":"1.20.1","type":"game"}]},
        {"id":3,"name":"hidden","type":"release","updated":1660000000,"private":true,"targets":[{"name":"minecraft","version":"1.20.1"}]},
        {"id":4,"name":"broken","type":"release","updated":1670000000,"targets":[]}]}"#;

    #[test]
    fn pack_details_become_pack_and_versions() {
        let raw: RawPack = serde_json::from_str(PACK).unwrap();
        let versions = serde_json::from_str::<RawPack>(PACK).unwrap().into_versions();
        let pack = raw.into_pack();
        assert_eq!(pack.name, "FTB Skies");
        assert_eq!(pack.icon_url.as_deref(), Some("https://x/logo.webp"));
        assert_eq!(pack.authors, ["FTB Team"]);
        assert_eq!(pack.updated, "2023-11-14T22:13:20+00:00");

        let ids: Vec<u64> = versions.iter().map(|version| version.id).collect();
        assert_eq!(ids, [2, 1]);
        assert_eq!(versions[0].loader, None);
        assert_eq!(versions[1].loader, Some(FtbLoaderRef { id: Loader::NeoForge, version: "47.1".into() }));
        let json = serde_json::to_value(&versions[1]).unwrap();
        assert_eq!(json["minecraftVersion"], "1.20.1");
        assert_eq!(json["type"], "release");
        assert_eq!(json["loader"]["id"], "neoForge");
    }

    #[test]
    fn manifest_files_parse_with_curseforge_ids() {
        let raw: RawManifest = serde_json::from_str(
            r#"{"status":"success","targets":[{"name":"minecraft","version":"1.20.1"},{"name":"forge","version":"47.2.0"}],
            "files":[{"path":"./mods","name":"a.jar","url":"https://x/a.jar","mirrors":["https://m/a.jar"],"sha1":"ab",
              "clientonly":false,"serveronly":false,"curseforge":{"project":"1","file":"2"}},
              {"path":"./mods/","name":"b.jar","url":"","serveronly":true}]}"#,
        )
        .unwrap();
        let (minecraft, loader) = targets_of(&raw.targets);
        assert_eq!(minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(loader.unwrap().id, Loader::Forge);
        assert_eq!(raw.files[0].curseforge, Some(FtbCurseForgeRef { project: "1".into(), file: "2".into() }));
        assert!(raw.files[1].serveronly && raw.files[1].url.is_empty());
        assert_eq!(relative_path(&raw.files[0]), "mods/a.jar");
        assert_eq!(relative_path(&raw.files[1]), "mods/b.jar");
    }

    #[test]
    fn root_files_and_error_status() {
        let file = FtbFile { path: "./".into(), name: "options.txt".into(), ..FtbFile::default() };
        assert_eq!(relative_path(&file), "options.txt");
        let ids: RawIds = serde_json::from_str(r#"{"status":"error","message":"nope"}"#).unwrap();
        assert!(ids.check().unwrap_err().to_string().contains("nope"));
    }
}
