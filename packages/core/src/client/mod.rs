//! Game clients: alternatives to the mod loaders (LabyMod, later others) layered on vanilla Minecraft.
//!
//! A client is an instance component whose uid is owned by the launcher, not by Prism meta. A
//! *provider* resolves it to a [`VersionFile`] with the client's own main class, classpath, JVM and
//! game arguments and required Java, plus [`ClientFiles`] that go into the game directory instead
//! of `libraries/`. [`apply`] then lets that file replace the launch parts of `net.minecraft`
//! (which keeps contributing the client jar, asset index and traits), so the vanilla libraries and
//! the loader dependencies (LWJGL 2 and the like) never mix with the client's own. Clients exclude
//! the mod loaders and each other. A second provider only adds a uid and a branch in [`load`].

pub mod labymod;

use std::path::{Path, PathBuf};

use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::download::{DownloadItem, download_all, sha1_file};
use crate::error::{Error, IoContext, Result};
use crate::meta::{Loader, MetaClient};
use crate::profile::{LaunchProfile, ResolvedComponent};
use crate::progress::ProgressFn;

/// The clients the UI can offer. The serialized name is also the key of the accepted terms in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Labymod4,
}

impl ClientKind {
    pub fn uid(self) -> &'static str {
        match self {
            Self::Labymod4 => labymod::UID,
        }
    }

    /// Key in `GlobalSettings::accepted_client_terms`.
    pub fn terms_key(self) -> &'static str {
        match self {
            Self::Labymod4 => "labymod4",
        }
    }

    /// Component version used when a client is added to an instance (the release channel).
    pub fn default_version(self) -> &'static str {
        match self {
            Self::Labymod4 => labymod::DEFAULT_CHANNEL,
        }
    }

    /// Minecraft versions the client currently supports.
    pub async fn supported_versions(self, meta: &MetaClient) -> Result<Vec<String>> {
        match self {
            Self::Labymod4 => labymod::supported_versions(meta, labymod::DEFAULT_CHANNEL).await,
        }
    }

    /// The text the user accepts before the client is first used.
    pub async fn terms(self, http: &reqwest::Client) -> Result<String> {
        match self {
            Self::Labymod4 => labymod::terms(http).await,
        }
    }
}

/// Uids of the supported clients.
pub const CLIENT_UIDS: [&str; 1] = [labymod::UID];

pub fn is_client_uid(uid: &str) -> bool {
    CLIENT_UIDS.contains(&uid)
}

/// A file a client needs inside the instance's game directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameFile {
    /// Relative to the game directory, `/`-separated.
    pub path: String,
    pub url: String,
    pub sha1: String,
}

/// Written once every file of a release is on disk; offline resolving uses the newest such release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMark {
    pub path: PathBuf,
    pub contents: String,
}

/// What a client puts into the game directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientFiles {
    pub files: Vec<GameFile>,
    pub mark: Option<InstalledMark>,
}

/// Resolves a client component for `minecraft_version`. Errors when the client does not support it.
pub async fn load(meta: &MetaClient, spec: ComponentSpec, minecraft_version: &str) -> Result<ResolvedComponent> {
    let file = match spec.uid.as_str() {
        labymod::UID => labymod::resolve(meta, &spec.version, minecraft_version).await?,
        uid => return Err(Error::Other(format!("`{uid}` is not a known client"))),
    };
    Ok(ResolvedComponent { spec, file })
}

/// Checks that the component list has at most one client and no mod loader next to it.
pub fn check_exclusive(specs: &[ComponentSpec]) -> Result<()> {
    let clients: Vec<&str> = specs.iter().map(|spec| spec.uid.as_str()).filter(|uid| is_client_uid(uid)).collect();
    if clients.len() > 1 {
        return Err(Error::Other(format!("only one client can be used, got {}", clients.join(" and "))));
    }
    let loaders = [Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge].map(Loader::uid);
    if let (Some(client), Some(loader)) =
        (clients.first(), specs.iter().find(|spec| loaders.contains(&spec.uid.as_str())))
    {
        return Err(Error::Other(format!("the client {client} cannot be combined with the mod loader {}", loader.uid)));
    }
    Ok(())
}

/// Strips the launch parts of `net.minecraft` that a client in the list replaces with its own.
pub fn apply(components: &mut [ResolvedComponent]) {
    if !components.iter().any(|component| is_client_uid(&component.spec.uid)) {
        return;
    }
    for component in components.iter_mut().filter(|component| component.spec.uid == MINECRAFT_UID) {
        let file = &mut component.file;
        file.requires.clear();
        file.libraries.clear();
        file.maven_files.clear();
        file.main_class = None;
        file.minecraft_arguments = None;
        file.jvm_args.clear();
        file.tweakers.clear();
        file.compatible_java_majors.clear();
        file.compatible_java_name = None;
    }
}

fn game_file_path(game_dir: &Path, file: &GameFile) -> PathBuf {
    file.path.split('/').fold(game_dir.to_owned(), |path, part| path.join(part))
}

/// Downloads the client's game-directory files that are missing or changed, then records the release as
/// installed. Call it after [`crate::install::install_game`].
pub async fn install_files(
    client: &reqwest::Client,
    game_dir: &Path,
    profile: &LaunchProfile,
    task: &str,
    progress: &ProgressFn,
) -> Result<()> {
    let files = &profile.client_files;
    let items = files
        .files
        .iter()
        .map(|file| DownloadItem {
            url: file.url.clone(),
            path: game_file_path(game_dir, file),
            sha1: Some(file.sha1.clone()),
        })
        .collect();
    download_all(client, items, task, "Client files", progress).await?;
    if let Some(mark) = &files.mark {
        if let Some(parent) = mark.path.parent() {
            tokio::fs::create_dir_all(parent).await.at(parent)?;
        }
        tokio::fs::write(&mark.path, &mark.contents).await.at(&mark.path)?;
    }
    Ok(())
}

/// Verifies the client's game-directory files for launching without network access.
pub async fn check_files(game_dir: &Path, profile: &LaunchProfile) -> Result<()> {
    for file in &profile.client_files.files {
        let path = game_file_path(game_dir, file);
        let matches = match sha1_file(&path).await {
            Ok(actual) => actual == file.sha1,
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if !matches {
            return Err(Error::Other(format!("the client file {} is missing or changed", path.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(uid: &str) -> ComponentSpec {
        ComponentSpec { uid: uid.to_owned(), version: "1".to_owned(), dependency_only: false }
    }

    #[test]
    fn a_client_excludes_loaders_and_other_clients() {
        assert!(check_exclusive(&[spec(MINECRAFT_UID), spec(labymod::UID)]).is_ok());
        assert!(check_exclusive(&[spec(MINECRAFT_UID), spec("net.fabricmc.fabric-loader")]).is_ok());
        assert!(check_exclusive(&[spec(MINECRAFT_UID), spec(labymod::UID), spec("net.neoforged")]).is_err());
        assert!(check_exclusive(&[spec(MINECRAFT_UID), spec(labymod::UID), spec(labymod::UID)]).is_err());
    }
}
