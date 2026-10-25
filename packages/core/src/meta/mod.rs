//! Client for Prism's meta server: package index, per-package version lists and version files.
//!
//! Every response is cached under `meta/` as the raw bytes and validated against the `sha256`
//! of its parent index. When the network fails, cached files are used.

mod types;

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

use crate::component::MINECRAFT_UID;
use crate::download::http_client;
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

pub use types::{
    AssetIndexRef, LoggingConfig, LoggingFile, MojangLaunch, PackageEntry, PackageIndex, Requirement, VersionEntry,
    VersionFile, VersionList,
};

/// Per-request limit for files outside Prism meta, so a dead host cannot stall a launch.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

const META_URL: &str = "https://meta.prismlauncher.org/v1/";

const MOJANG_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loader {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub fn uid(self) -> &'static str {
        match self {
            Self::Fabric => "net.fabricmc.fabric-loader",
            Self::Quilt => "org.quiltmc.quilt-loader",
            Self::Forge => "net.minecraftforge",
            Self::NeoForge => "net.neoforged",
        }
    }
}

pub struct MetaClient {
    http: reqwest::Client,
    dir: PathBuf,
    offline: bool,
    /// Fetched once per client; create a new client to refresh it.
    packages: OnceCell<PackageIndex>,
}

#[derive(Deserialize)]
struct MojangManifest {
    versions: Vec<MojangManifestEntry>,
}

#[derive(Deserialize)]
struct MojangManifestEntry {
    id: String,
    url: String,
    sha1: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// uid and version end up in cache file names, so they must not escape the cache directory.
fn check_path_component(value: &str) -> Result<()> {
    if value.is_empty() || value.contains(['/', '\\']) || value == ".." || value == "." {
        return Err(Error::Other(format!("invalid meta name `{value}`")));
    }
    Ok(())
}

async fn read_cached(path: &Path) -> Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).at(path),
    }
}

async fn write_cached(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.at(parent)?;
    }
    let tmp = path.with_extension("part");
    tokio::fs::write(&tmp, bytes).await.at(&tmp)?;
    tokio::fs::rename(&tmp, path).await.at(path)
}

impl MetaClient {
    /// In `offline` mode only cached files are used and nothing is validated or fetched.
    pub fn new(paths: &Paths, offline: bool) -> Self {
        Self { http: http_client(), dir: paths.meta_dir(), offline, packages: OnceCell::new() }
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let http = |source| Error::Http { url: url.to_owned(), source };
        let response = self.http.get(url).send().await.map_err(http)?;
        let bytes = response.error_for_status().map_err(http)?.bytes().await.map_err(http)?;
        Ok(bytes.to_vec())
    }

    /// The cached file when it is at most `max_age` old (or when offline), otherwise the response of `url`,
    /// cached under `relative`. A failed request falls back to whatever is cached. For files outside Prism
    /// meta (see `crate::client`), which have no index to validate against.
    pub(crate) async fn fetch_url(&self, url: &str, relative: &str, max_age: Duration) -> Result<Vec<u8>> {
        let cache_path = self.dir.join(relative);
        let cached = read_cached(&cache_path).await?;
        if let Some(bytes) = &cached {
            let age = tokio::fs::metadata(&cache_path).await.at(&cache_path)?.modified().ok().and_then(|time| time.elapsed().ok());
            if self.offline || age.is_some_and(|age| age <= max_age) {
                return Ok(bytes.clone());
            }
        }
        if self.offline {
            return Err(Error::Other(format!("{relative} is not cached and offline mode is on")));
        }
        let http = |source| Error::Http { url: url.to_owned(), source };
        let request = self.http.get(url).timeout(REQUEST_TIMEOUT).send().await;
        let downloaded = match request {
            Ok(response) => match response.error_for_status() {
                Ok(response) => response.bytes().await.map_err(http),
                Err(source) => Err(http(source)),
            },
            Err(source) => Err(http(source)),
        };
        match (downloaded, cached) {
            (Ok(bytes), _) => {
                write_cached(&cache_path, &bytes).await?;
                Ok(bytes.to_vec())
            }
            (Err(error), Some(bytes)) => {
                tracing::warn!("using cached {relative} because {url} is unreachable: {error}");
                Ok(bytes)
            }
            (Err(error), None) => Err(error),
        }
    }

    pub(crate) fn is_offline(&self) -> bool {
        self.offline
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the cached file when its hash matches, otherwise fetches and caches it.
    /// A failed fetch falls back to whatever is cached.
    async fn fetch(&self, relative: &str, expected_sha256: Option<&str>) -> Result<Vec<u8>> {
        let cache_path = self.dir.join(relative);
        let cached = read_cached(&cache_path).await?;

        if let Some(bytes) = &cached
            && (self.offline || expected_sha256.is_some_and(|expected| sha256_hex(bytes) == expected))
        {
            return Ok(bytes.clone());
        }
        if self.offline {
            return Err(Error::Other(format!("{relative} is not cached and offline mode is on")));
        }

        let url = format!("{META_URL}{relative}");
        let bytes = match self.download(&url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return match cached {
                    Some(bytes) => {
                        tracing::warn!("using cached {relative} because the meta server is unreachable: {error}");
                        Ok(bytes)
                    }
                    None => Err(error),
                };
            }
        };

        if let Some(expected) = expected_sha256 {
            let actual = sha256_hex(&bytes);
            if actual != expected {
                return Err(Error::HashMismatch { url, expected: expected.to_owned(), actual });
            }
        }
        write_cached(&cache_path, &bytes).await?;
        Ok(bytes)
    }

    pub async fn package_index(&self) -> Result<&PackageIndex> {
        self.packages
            .get_or_try_init(|| async { Ok(serde_json::from_slice(&self.fetch("index.json", None).await?)?) })
            .await
    }

    pub async fn version_list(&self, uid: &str) -> Result<VersionList> {
        check_path_component(uid)?;
        let expected = if self.offline {
            None
        } else {
            let package = self.package_index().await?.packages.iter().find(|package| package.uid == uid);
            let package = package.ok_or_else(|| Error::Other(format!("unknown meta package `{uid}`")))?;
            Some(package.sha256.as_str())
        };
        let bytes = self.fetch(&format!("{uid}/index.json"), expected).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn version_file(&self, uid: &str, version: &str) -> Result<VersionFile> {
        check_path_component(uid)?;
        check_path_component(version)?;
        let expected = if self.offline {
            None
        } else {
            let list = self.version_list(uid).await?;
            let entry = list.versions.into_iter().find(|entry| entry.version == version);
            Some(entry.ok_or_else(|| Error::Other(format!("unknown version {version} of `{uid}`")))?.sha256)
        };
        let bytes = self.fetch(&format!("{uid}/{version}.json"), expected.as_deref()).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// All Minecraft versions, newest first, each with its `type`.
    pub async fn minecraft_versions(&self) -> Result<Vec<VersionEntry>> {
        Ok(self.version_list(MINECRAFT_UID).await?.versions)
    }

    /// The launch parts of Mojang's own version JSON. Prism starts pre-1.6 versions through a launcher
    /// jar of its own; Mojang's JSON starts them through `launchwrapper` and its tweakers instead.
    /// The JSON of a version never changes, so a cached copy is used without a request.
    pub async fn mojang_launch(&self, version: &str) -> Result<MojangLaunch> {
        check_path_component(version)?;
        let cache_path = self.dir.join("mojang").join(format!("{version}.json"));
        if let Some(bytes) = read_cached(&cache_path).await? {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        if self.offline {
            return Err(Error::Other(format!("Mojang's version file for {version} is not cached and offline mode is on")));
        }

        let manifest: MojangManifest = serde_json::from_slice(&self.download(MOJANG_MANIFEST_URL).await?)?;
        let entry = manifest
            .versions
            .into_iter()
            .find(|entry| entry.id == version)
            .ok_or_else(|| Error::Other(format!("Mojang's version manifest has no version {version}")))?;
        let bytes = self.download(&entry.url).await?;
        let actual = hex::encode(Sha1::digest(&bytes));
        if actual != entry.sha1 {
            return Err(Error::HashMismatch { url: entry.url, expected: entry.sha1, actual });
        }
        let launch = serde_json::from_slice(&bytes)?;
        write_cached(&cache_path, &bytes).await?;
        Ok(launch)
    }

    /// Loader versions usable with `mc_version`, best default first: recommended, then releases
    /// newest first, then the rest. Forge and NeoForge are pinned to one Minecraft version through
    /// `requires`; Fabric and Quilt work with every version.
    pub async fn loader_versions(&self, loader: Loader, mc_version: &str) -> Result<Vec<VersionEntry>> {
        let mut versions = self.version_list(loader.uid()).await?.versions;
        if matches!(loader, Loader::Forge | Loader::NeoForge) {
            versions.retain(|entry| {
                entry.requires.iter().any(|requirement| {
                    requirement.uid == MINECRAFT_UID && requirement.equals.as_deref() == Some(mc_version)
                })
            });
        }
        versions.sort_by_key(|entry| {
            (
                !entry.recommended,
                entry.kind.as_deref().is_some_and(|kind| kind != "release"),
                Reverse(entry.release_time),
            )
        });
        Ok(versions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str, kind: Option<&str>, recommended: bool, time: &str, mc: Option<&str>) -> String {
        let requires =
            mc.map(|mc| format!(r#"[{{"uid":"net.minecraft","equals":"{mc}"}}]"#)).unwrap_or_else(|| "[]".to_owned());
        let kind = kind.map(|kind| format!(r#""type":"{kind}","#)).unwrap_or_default();
        format!(
            r#"{{"version":"{version}",{kind}"recommended":{recommended},"releaseTime":"{time}","requires":{requires},"sha256":"00"}}"#
        )
    }

    async fn client_with_list(uid: &str, entries: &[String]) -> (tempfile::TempDir, MetaClient) {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path());
        let list = format!(r#"{{"uid":"{uid}","versions":[{}]}}"#, entries.join(","));
        write_cached(&paths.meta_dir().join(uid).join("index.json"), list.as_bytes()).await.unwrap();
        (root, MetaClient::new(&paths, true))
    }

    #[tokio::test]
    async fn cached_mojang_launch_is_used_offline() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path());
        let json = r#"{"id":"b1.7.3","mainClass":"net.minecraft.launchwrapper.Launch",
            "minecraftArguments":"${auth_player_name} ${auth_session}","libraries":[{"name":"net.minecraft:launchwrapper:1.5"}]}"#;
        write_cached(&paths.meta_dir().join("mojang/b1.7.3.json"), json.as_bytes()).await.unwrap();
        let client = MetaClient::new(&paths, true);

        let launch = client.mojang_launch("b1.7.3").await.unwrap();
        assert_eq!(launch.main_class, "net.minecraft.launchwrapper.Launch");
        assert_eq!(launch.minecraft_arguments, "${auth_player_name} ${auth_session}");
        assert_eq!(launch.libraries[0].name.to_string(), "net.minecraft:launchwrapper:1.5");
        assert!(client.mojang_launch("a1.2.6").await.is_err());
    }

    #[tokio::test]
    async fn forge_versions_are_filtered_by_minecraft_version() {
        let (_root, client) = client_with_list(
            "net.minecraftforge",
            &[
                entry("47.4.28", None, true, "2026-01-02T00:00:00+00:00", Some("1.20.1")),
                entry("47.4.27", None, false, "2026-01-01T00:00:00+00:00", Some("1.20.1")),
                entry("52.0.1", None, true, "2026-01-03T00:00:00+00:00", Some("1.21.1")),
            ],
        )
        .await;
        let versions = client.loader_versions(Loader::Forge, "1.20.1").await.unwrap();
        let names: Vec<_> = versions.iter().map(|entry| entry.version.as_str()).collect();
        assert_eq!(names, ["47.4.28", "47.4.27"]);
    }

    #[tokio::test]
    async fn fabric_versions_put_recommended_then_newest_release_first() {
        let (_root, client) = client_with_list(
            "net.fabricmc.fabric-loader",
            &[
                entry("0.20.2-beta", Some("snapshot"), false, "2026-03-01T00:00:00+00:00", None),
                entry("0.20.1", Some("release"), false, "2026-02-01T00:00:00+00:00", None),
                entry("0.19.0", Some("release"), true, "2026-01-01T00:00:00+00:00", None),
                entry("0.20.0", Some("release"), false, "2026-01-15T00:00:00+00:00", None),
            ],
        )
        .await;
        let versions = client.loader_versions(Loader::Fabric, "1.21.4").await.unwrap();
        let names: Vec<_> = versions.iter().map(|entry| entry.version.as_str()).collect();
        assert_eq!(names, ["0.19.0", "0.20.1", "0.20.0", "0.20.2-beta"]);
    }

    #[tokio::test]
    async fn offline_mode_reports_missing_cache() {
        let root = tempfile::tempdir().unwrap();
        let client = MetaClient::new(&Paths::new(root.path()), true);
        let error = client.version_file("net.minecraft", "1.21.4").await.unwrap_err();
        assert!(error.to_string().contains("not cached"), "{error}");
    }

    #[tokio::test]
    async fn path_traversal_in_names_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let client = MetaClient::new(&Paths::new(root.path()), true);
        assert!(client.version_file("net.minecraft", "../x").await.is_err());
        assert!(client.version_list("a/b").await.is_err());
    }
}
