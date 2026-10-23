//! Installing a project version into an instance (with its required dependencies) and checking
//! installed resources for updates.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};

use super::modrinth::HashAlgorithm;
use super::packwiz::{self, PwMod};
use super::{Clients, DependencyKind, Loader, Platform, ProjectVersion, ResourceKind, VersionFile};
use crate::download::{Checksum, download_checked};
use crate::error::{Error, IoContext, Result};
use crate::progress::{ProgressEvent, ProgressFn};

/// Prism stops following dependency chains at this depth.
const MAX_DEPENDENCY_DEPTH: usize = 20;
const PARALLEL_UPDATE_LOOKUPS: usize = 8;

/// Fabric to Quilt replacements `(platform, fabric project id, quilt project id)`; Quilt runs Fabric
/// mods, so the Fabric project is the fallback when the Quilt one has no matching version.
const QUILT_OVERRIDES: [(Platform, &str, &str); 4] = [
    (Platform::Modrinth, "P7dR8mSH", "qvIfYCYJ"),
    (Platform::Modrinth, "Ha28R6CL", "lwVhp9o5"),
    (Platform::CurseForge, "306612", "634179"),
    (Platform::CurseForge, "308769", "720410"),
];

/// The instance a resource is installed into.
#[derive(Debug, Clone)]
pub struct InstallContext {
    pub game_dir: PathBuf,
    pub game_version: String,
    /// Loaders the instance accepts, preferred first (a Quilt instance usually lists Quilt and Fabric).
    /// Only applied to mods; other resource kinds are not loader-specific.
    pub loaders: Vec<Loader>,
}

impl InstallContext {
    fn loaders_for(&self, kind: ResourceKind) -> &[Loader] {
        match kind {
            ResourceKind::Mod => &self.loaders,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledResource {
    pub platform: Platform,
    pub project_id: String,
    pub name: String,
    pub file_name: String,
    pub is_dependency: bool,
}

/// A file the author does not allow third parties to download; the user has to fetch it from
/// `download_page` and drop it into `target_path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedFile {
    pub platform: Platform,
    pub project_id: String,
    pub name: String,
    pub file_name: String,
    /// Path relative to the game dir, e.g. `mods/example.jar`.
    pub target_path: String,
    pub download_page: Option<String>,
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub installed: Vec<InstalledResource>,
    pub blocked: Vec<BlockedFile>,
    /// Required dependencies for which no compatible version exists, as `platform:project id`.
    pub unresolved_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    /// Current file name (without `.disabled`).
    pub file_name: String,
    pub name: String,
    pub current_version: String,
    pub latest: ProjectVersion,
}

/// Replaces characters Windows forbids in file names; the result must stay a plain name.
fn sanitize_file_name(name: &str) -> Result<String> {
    let clean: String = name
        .chars()
        .filter(|c| !c.is_control() && !"\\/:*?\"<>|".contains(*c))
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." {
        return Err(Error::Other(format!("{name:?} is not a usable file name")));
    }
    Ok(clean)
}

pub(super) fn checksum_of(file: &VersionFile) -> Option<Checksum> {
    match (&file.hashes.sha512, &file.hashes.sha1) {
        (Some(hash), _) => Some(Checksum::Sha512(hash.to_ascii_lowercase())),
        (None, Some(hash)) => Some(Checksum::Sha1(hash.to_ascii_lowercase())),
        (None, None) => None,
    }
}

/// Installs `version` and, recursively, its required dependencies that are not installed yet.
/// A file without a download URL is returned in `blocked` instead of failing the install.
/// Installing a newer version of an installed project replaces its old file.
pub async fn install_version(
    clients: &Clients,
    ctx: &InstallContext,
    kind: ResourceKind,
    version: &ProjectVersion,
    progress: &ProgressFn,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    let mut handled: HashSet<(Platform, String)> =
        HashSet::from([(version.platform, version.project_id.clone())]);
    let mut queue = VecDeque::from([(version.clone(), 0_usize)]);

    while let Some((version, depth)) = queue.pop_front() {
        install_one(
            clients,
            ctx,
            kind,
            &version,
            depth > 0,
            &mut report,
            progress,
        )
        .await?;
        if depth >= MAX_DEPENDENCY_DEPTH {
            continue;
        }
        for dependency in version
            .dependencies
            .iter()
            .filter(|d| d.kind == DependencyKind::Required)
        {
            let platform = version.platform;
            // Only Modrinth dependencies can pin a version.
            let pinned_id = dependency
                .version_id
                .as_deref()
                .filter(|_| platform == Platform::Modrinth);
            let project_id = match (&dependency.project_id, pinned_id) {
                (Some(project_id), _) => project_id.clone(),
                (None, Some(version_id)) => clients.modrinth.version(version_id).await?.project_id,
                (None, None) => continue,
            };
            if !handled.insert((platform, project_id.clone()))
                || is_installed(ctx, kind, platform, &project_id)?
            {
                continue;
            }
            let resolved = match pinned_id {
                Some(version_id) => Some(clients.modrinth.version(version_id).await?),
                None => latest_compatible(clients, ctx, kind, platform, &project_id).await?,
            };
            match resolved {
                Some(dependency_version) => {
                    handled.insert((platform, dependency_version.project_id.clone()));
                    queue.push_back((dependency_version, depth + 1));
                }
                None => report
                    .unresolved_dependencies
                    .push(format!("{platform:?}:{project_id}")),
            }
        }
    }
    Ok(report)
}

/// True when the project or, for Fabric-to-Quilt substitutes, its replacement is already installed.
fn is_installed(
    ctx: &InstallContext,
    kind: ResourceKind,
    platform: Platform,
    project_id: &str,
) -> Result<bool> {
    let index_dir = packwiz::index_dir(&ctx.game_dir, kind);
    for id in candidate_ids(ctx, platform, project_id) {
        if packwiz::find_by_project(&index_dir, platform, &id)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Project ids to try for a dependency, in order: the Quilt replacement (for Quilt instances), then itself.
fn candidate_ids(ctx: &InstallContext, platform: Platform, project_id: &str) -> Vec<String> {
    let quilt = QUILT_OVERRIDES
        .iter()
        .find(|(p, fabric, _)| *p == platform && *fabric == project_id)
        .filter(|_| ctx.loaders.contains(&Loader::Quilt))
        .map(|(_, _, quilt)| (*quilt).to_owned());
    quilt.into_iter().chain([project_id.to_owned()]).collect()
}

/// Newest version of a dependency for the instance, preferring the Quilt replacement when there is one.
async fn latest_compatible(
    clients: &Clients,
    ctx: &InstallContext,
    kind: ResourceKind,
    platform: Platform,
    project_id: &str,
) -> Result<Option<ProjectVersion>> {
    let ids = candidate_ids(ctx, platform, project_id);
    for (index, id) in ids.iter().enumerate() {
        let lookup = clients
            .versions(platform, id, &ctx.game_version, ctx.loaders_for(kind))
            .await;
        let versions = match lookup {
            Ok(versions) => versions,
            // A failed replacement lookup falls back to the original project; the last lookup must succeed.
            Err(_) if index + 1 < ids.len() => continue,
            Err(error) => return Err(error),
        };
        if let Some(latest) = versions
            .into_iter()
            .max_by(|a, b| a.published.cmp(&b.published))
        {
            return Ok(Some(latest));
        }
    }
    Ok(None)
}

async fn install_one(
    clients: &Clients,
    ctx: &InstallContext,
    kind: ResourceKind,
    version: &ProjectVersion,
    is_dependency: bool,
    report: &mut InstallReport,
    progress: &ProgressFn,
) -> Result<()> {
    let file = version
        .install_file()
        .ok_or_else(|| Error::Other(format!("version {} has no files", version.id)))?;
    let file_name = sanitize_file_name(&file.filename)?;
    let project = clients
        .project(version.platform, &version.project_id)
        .await?;

    let Some(url) = &file.url else {
        report.blocked.push(BlockedFile {
            platform: version.platform,
            project_id: version.project_id.clone(),
            name: project.title,
            target_path: format!("{}/{file_name}", kind.folder()),
            file_name,
            download_page: file.download_page.clone(),
            sha1: file.hashes.sha1.clone(),
        });
        return Ok(());
    };

    let report_progress = |done| {
        progress(ProgressEvent {
            task: format!("install:{}", version.project_id),
            stage: format!("Installing {}", project.title),
            done,
            total: 1,
        })
    };
    report_progress(0);
    let destination = ctx.game_dir.join(kind.folder()).join(&file_name);
    download_checked(&clients.http, url, &destination, checksum_of(file).as_ref()).await?;

    let index_dir = packwiz::index_dir(&ctx.game_dir, kind);
    let previous = packwiz::find_by_project(&index_dir, version.platform, &version.project_id)?;
    let index_path = packwiz::write(
        &index_dir,
        &project.slug,
        &PwMod::new(&project, version, file, false)?,
    )?;

    if let Some(previous) = previous {
        let same_index = previous.path.file_name().is_some_and(|name| {
            index_path
                .file_name()
                .is_some_and(|new| new.eq_ignore_ascii_case(name))
        });
        if !same_index {
            std::fs::remove_file(&previous.path).at(&previous.path)?;
        }
        if previous.data.filename != file_name {
            let old = ctx
                .game_dir
                .join(kind.folder())
                .join(&previous.data.filename);
            let old_disabled = old.with_file_name(format!("{}.disabled", previous.data.filename));
            for path in [old, old_disabled].into_iter().filter(|path| path.exists()) {
                std::fs::remove_file(&path).at(&path)?;
            }
        }
    }

    report_progress(1);
    report.installed.push(InstalledResource {
        platform: version.platform,
        project_id: version.project_id.clone(),
        name: project.title,
        file_name,
        is_dependency,
    });
    Ok(())
}

/// Checks every installed resource of `kind` that has a packwiz index (and no update lock) for a newer
/// version. Modrinth entries are checked in one batch per hash type, CurseForge ones per project.
/// CurseForge entries are skipped when `clients` has no CurseForge client.
pub async fn check_updates(
    clients: &Clients,
    ctx: &InstallContext,
    kind: ResourceKind,
) -> Result<Vec<AvailableUpdate>> {
    let entries = packwiz::list(&packwiz::index_dir(&ctx.game_dir, kind))?;
    let loaders = ctx.loaders_for(kind);
    let game_versions = [ctx.game_version.clone()];
    let mut updates = Vec::new();

    let mut by_algorithm: HashMap<bool, Vec<&packwiz::IndexEntry>> = HashMap::new();
    let mut curseforge_entries = Vec::new();
    for entry in entries.iter().filter(|entry| !entry.data.lock_update) {
        match entry.data.origin() {
            Some((Platform::Modrinth, ..)) if !entry.data.download.hash.is_empty() => {
                by_algorithm
                    .entry(entry.data.download.hash_format == "sha512")
                    .or_default()
                    .push(entry);
            }
            Some((Platform::CurseForge, project_id, file_id)) => {
                curseforge_entries.push((entry, project_id, file_id))
            }
            _ => {}
        }
    }

    for (is_sha512, group) in by_algorithm {
        let algorithm = if is_sha512 {
            HashAlgorithm::Sha512
        } else {
            HashAlgorithm::Sha1
        };
        let hashes: Vec<String> = group
            .iter()
            .map(|entry| entry.data.download.hash.clone())
            .collect();
        let latest = clients
            .modrinth
            .latest_versions_by_hashes(&hashes, algorithm, loaders, &game_versions)
            .await?;
        for entry in group {
            let current = entry
                .data
                .origin()
                .map(|(.., version)| version)
                .unwrap_or_default();
            if let Some(version) = latest
                .get(&entry.data.download.hash)
                .filter(|version| version.id != current)
            {
                updates.push(update_of(entry, current, version.clone()));
            }
        }
    }

    if let Some(curseforge) = &clients.curseforge {
        let found: Vec<Option<AvailableUpdate>> = stream::iter(curseforge_entries)
            .map(|(entry, project_id, file_id)| async move {
                let versions = curseforge
                    .versions(&project_id, Some(&ctx.game_version), loaders)
                    .await?;
                let latest = versions
                    .into_iter()
                    .max_by(|a, b| a.published.cmp(&b.published));
                Ok::<_, Error>(
                    latest
                        .filter(|latest| latest.id != file_id)
                        .map(|latest| update_of(entry, file_id, latest)),
                )
            })
            .buffer_unordered(PARALLEL_UPDATE_LOOKUPS)
            .try_collect()
            .await?;
        updates.extend(found.into_iter().flatten());
    }

    updates.sort_by_key(|update| update.name.to_lowercase());
    Ok(updates)
}

fn update_of(
    entry: &packwiz::IndexEntry,
    current_version: String,
    latest: ProjectVersion,
) -> AvailableUpdate {
    AvailableUpdate {
        file_name: entry.data.filename.clone(),
        name: entry.data.name.clone(),
        current_version,
        latest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(loaders: &[Loader]) -> InstallContext {
        InstallContext {
            game_dir: PathBuf::new(),
            game_version: "1.21.4".into(),
            loaders: loaders.to_vec(),
        }
    }

    #[test]
    fn file_names_lose_forbidden_characters_and_traversal() {
        assert_eq!(
            sanitize_file_name("sodium-0.6+mc1.21.jar").unwrap(),
            "sodium-0.6+mc1.21.jar"
        );
        assert_eq!(sanitize_file_name("a/../b:c.jar").unwrap(), "a..bc.jar");
        assert!(sanitize_file_name("..").is_err());
        assert!(sanitize_file_name("///").is_err());
    }

    #[test]
    fn quilt_instances_prefer_the_quilt_api_with_fabric_fallback() {
        assert_eq!(
            candidate_ids(
                &ctx(&[Loader::Quilt, Loader::Fabric]),
                Platform::Modrinth,
                "P7dR8mSH"
            ),
            ["qvIfYCYJ", "P7dR8mSH"]
        );
        assert_eq!(
            candidate_ids(&ctx(&[Loader::Fabric]), Platform::Modrinth, "P7dR8mSH"),
            ["P7dR8mSH"]
        );
        assert_eq!(
            candidate_ids(&ctx(&[Loader::Quilt]), Platform::CurseForge, "306612"),
            ["634179", "306612"]
        );
    }
}
