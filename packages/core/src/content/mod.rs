//! Mods and other content from Modrinth and CurseForge: search, install, update, local resources
//! and modpack import. The packwiz `.pw.toml` index keeps the origin of every installed file.

pub mod curseforge;
mod install;
mod local;
mod modpack;
pub mod modrinth;
pub mod murmur2;
pub mod packwiz;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub(crate) async fn send_json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T> {
    let http = |source: reqwest::Error| Error::Http {
        url: source.url().map_or_else(String::new, ToString::to_string),
        source,
    };
    let response = request
        .send()
        .await
        .map_err(http)?
        .error_for_status()
        .map_err(http)?;
    response.json().await.map_err(http)
}

pub use install::{
    AvailableUpdate, BlockedFile, InstallContext, InstallReport, InstalledResource, check_updates,
    install_version,
};
pub use local::{
    LocalResource, ResourceMetadata, delete_resource, parse_mod_jar, scan_resources, set_enabled,
};
pub use modpack::{
    ModpackFormat, ModpackInstallReport, ModpackManifest, ModpackSource, inspect_modpack,
    install_modpack,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Modrinth,
    CurseForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Mod,
    ResourcePack,
    ShaderPack,
    DataPack,
    Modpack,
    World,
}

impl ResourceKind {
    /// Folder under the instance game dir.
    pub fn folder(self) -> &'static str {
        match self {
            ResourceKind::Mod | ResourceKind::Modpack => "mods",
            ResourceKind::ResourcePack => "resourcepacks",
            ResourceKind::ShaderPack => "shaderpacks",
            ResourceKind::DataPack => "datapacks",
            ResourceKind::World => "saves",
        }
    }

    fn curseforge_class_id(self) -> u32 {
        match self {
            ResourceKind::Mod => 6,
            ResourceKind::ResourcePack => 12,
            ResourceKind::World => 17,
            ResourceKind::ShaderPack => 6552,
            ResourceKind::Modpack => 4471,
            ResourceKind::DataPack => 6945,
        }
    }

    fn from_curseforge_class_id(id: u32) -> Option<Self> {
        [
            ResourceKind::Mod,
            ResourceKind::ResourcePack,
            ResourceKind::World,
            ResourceKind::ShaderPack,
            ResourceKind::Modpack,
            ResourceKind::DataPack,
        ]
        .into_iter()
        .find(|kind| kind.curseforge_class_id() == id)
    }

    fn modrinth_project_type(self) -> Result<&'static str> {
        match self {
            ResourceKind::Mod => Ok("mod"),
            ResourceKind::ResourcePack => Ok("resourcepack"),
            ResourceKind::ShaderPack => Ok("shader"),
            ResourceKind::DataPack => Ok("datapack"),
            ResourceKind::Modpack => Ok("modpack"),
            ResourceKind::World => Err(Error::Other("Modrinth has no worlds".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Loader {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub fn modrinth_id(self) -> &'static str {
        match self {
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::Forge => "forge",
            Loader::NeoForge => "neoforge",
        }
    }

    pub fn from_modrinth_id(id: &str) -> Option<Self> {
        [
            Loader::Fabric,
            Loader::Quilt,
            Loader::Forge,
            Loader::NeoForge,
        ]
        .into_iter()
        .find(|loader| loader.modrinth_id() == id.to_ascii_lowercase())
    }

    /// CurseForge `modLoaderType`.
    pub fn curseforge_id(self) -> u32 {
        match self {
            Loader::Forge => 1,
            Loader::Fabric => 4,
            Loader::Quilt => 5,
            Loader::NeoForge => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchSort {
    #[default]
    Relevance,
    Downloads,
    Follows,
    Newest,
    Updated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    pub kind: ResourceKind,
    pub game_version: Option<String>,
    pub loaders: Vec<Loader>,
    /// Ids as returned by `categories()` of the platform: names on Modrinth, numeric ids on CurseForge.
    pub categories: Vec<String>,
    pub sort: SearchSort,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub platform: Platform,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub offset: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Client,
    Server,
    Both,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub platform: Platform,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// Long description (markdown on Modrinth); `None` on CurseForge, where it is a separate HTML call.
    pub body: Option<String>,
    pub icon_url: Option<String>,
    pub kind: Option<ResourceKind>,
    pub side: Option<Side>,
    pub authors: Vec<String>,
    pub categories: Vec<String>,
    pub website_url: Option<String>,
    pub issues_url: Option<String>,
    pub source_url: Option<String>,
    pub wiki_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseType {
    Release,
    Beta,
    Alpha,
}

impl ReleaseType {
    pub fn as_str(self) -> &'static str {
        match self {
            ReleaseType::Release => "release",
            ReleaseType::Beta => "beta",
            ReleaseType::Alpha => "alpha",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "release" => Some(ReleaseType::Release),
            "beta" => Some(ReleaseType::Beta),
            "alpha" => Some(ReleaseType::Alpha),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionFile {
    /// `None` when the author disabled third-party downloads (CurseForge); see `download_page`.
    pub url: Option<String>,
    /// Website page where the file can be downloaded by hand, set when `url` is `None`.
    pub download_page: Option<String>,
    pub filename: String,
    pub size: u64,
    pub hashes: Hashes,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyKind {
    Required,
    Optional,
    Embedded,
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVersion {
    pub platform: Platform,
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub release_type: ReleaseType,
    pub game_versions: Vec<String>,
    pub loaders: Vec<Loader>,
    pub files: Vec<VersionFile>,
    pub dependencies: Vec<Dependency>,
    /// RFC 3339 timestamp; sorts chronologically as a string.
    pub published: String,
}

impl ProjectVersion {
    /// The file to install: the primary one, else the last.
    pub fn install_file(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .find(|file| file.primary)
            .or(self.files.last())
    }
}

/// A category or tag the platform can filter by; `id` goes into `SearchQuery::categories`.
#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub id: String,
    pub name: String,
}

/// API clients for both platforms. CurseForge is optional because it needs an API key.
#[derive(Debug, Clone)]
pub struct Clients {
    /// Plain HTTP client used for file downloads.
    pub http: reqwest::Client,
    pub modrinth: modrinth::Client,
    pub curseforge: Option<curseforge::Client>,
}

impl Clients {
    /// `http` should come from `download::http_client()`. CurseForge is enabled by passing its API key.
    pub fn new(http: reqwest::Client, curseforge_api_key: Option<&str>) -> Result<Self> {
        Ok(Self {
            modrinth: modrinth::Client::new(http.clone()),
            curseforge: curseforge_api_key
                .map(|key| curseforge::Client::new(http.clone(), key))
                .transpose()?,
            http,
        })
    }

    fn require_curseforge(&self) -> Result<&curseforge::Client> {
        self.curseforge.as_ref().ok_or_else(|| {
            Error::Other("a CurseForge API key is required for this operation".into())
        })
    }

    pub async fn project(&self, platform: Platform, id: &str) -> Result<Project> {
        match platform {
            Platform::Modrinth => self.modrinth.project(id).await,
            Platform::CurseForge => self.require_curseforge()?.project(id).await,
        }
    }

    /// Versions of a project compatible with `game_version` and any of `loaders` (all when empty).
    pub async fn versions(
        &self,
        platform: Platform,
        project_id: &str,
        game_version: &str,
        loaders: &[Loader],
    ) -> Result<Vec<ProjectVersion>> {
        match platform {
            Platform::Modrinth => {
                self.modrinth
                    .versions(project_id, &[game_version.to_owned()], loaders)
                    .await
            }
            Platform::CurseForge => {
                self.require_curseforge()?
                    .versions(project_id, Some(game_version), loaders)
                    .await
            }
        }
    }
}
