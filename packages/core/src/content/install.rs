//! Installing a project version into an instance (with its required dependencies) and checking
//! installed resources for updates.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use super::modcompat::{self, Descriptor, Provided};

use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};

use super::modrinth::HashAlgorithm;
use super::murmur2;
use super::packwiz::{self, PwMod};
use super::{Clients, DependencyKind, Loader, Platform, Project, ProjectVersion, ResourceKind, VersionFile};
use crate::download::{Checksum, download_checked, sha1_file};
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
    /// Unresolved catalog dependencies (`platform:project id`) or descriptor mod IDs.
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
    let mut queue = VecDeque::from([(version.clone(), kind, 0_usize)]);

    while let Some((version, kind, depth)) = queue.pop_front() {
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
            if !handled.insert((platform, project_id.clone())) {
                continue;
            }
            // A dependency goes into the folder of its own project type, not its parent's.
            let dependency_kind = clients.project(platform, &project_id).await?.kind;
            let Some(dependency_kind) = dependency_kind.filter(|dependency_kind| can_depend_on(kind, *dependency_kind)) else {
                report.unresolved_dependencies.push(format!("{platform:?}:{project_id}"));
                continue;
            };
            if is_installed(ctx, dependency_kind, platform, &project_id)? {
                continue;
            }
            let resolved = match pinned_id {
                Some(version_id) => Some(clients.modrinth.version(version_id).await?),
                None => latest_compatible(clients, ctx, dependency_kind, platform, &project_id).await?,
            };
            match resolved {
                Some(dependency_version) => {
                    handled.insert((platform, dependency_version.project_id.clone()));
                    queue.push_back((dependency_version, dependency_kind, depth + 1));
                }
                None => report
                    .unresolved_dependencies
                    .push(format!("{platform:?}:{project_id}")),
            }
        }
    }
    Ok(report)
}

// Local mod IDs are not catalog project IDs. Catalog results are only candidates until their
// downloaded descriptors prove that they satisfy every parent's requirements.
const MAX_LOCAL_DEPENDENCY_INSTALLS: usize = 24;
const MAX_LOCAL_DEPENDENCY_DOWNLOADS: usize = 48;
const MAX_LOCAL_DEPENDENCY_LOOKUPS: usize = 32;
const MAX_LOCAL_DEPENDENCY_PROJECTS: usize = 12;
const MAX_LOCAL_DEPENDENCY_VERSIONS: usize = 8;

type ModRequirements = BTreeMap<String, Vec<(Loader, Vec<String>)>>;

struct DependencyJar {
    path: PathBuf,
    enabled: bool,
    descriptors: Vec<Descriptor>,
}

struct ModDependencyState {
    jars: Vec<DependencyJar>,
    requirements: ModRequirements,
    /// Includes disabled and other-loader providers: never install a duplicate beside them.
    protected: HashSet<String>,
    missing: Vec<String>,
}

fn dependency_descriptor(descriptors: &[Descriptor], loader: Loader) -> Option<&Descriptor> {
    descriptors
        .iter()
        .find(|descriptor| descriptor.loader == loader)
        .or_else(|| {
            descriptors
                .iter()
                .find(|descriptor| modcompat::accepts(loader, descriptor.loader))
        })
}

fn provides_requirement(provided: &Provided, constraints: &[(Loader, Vec<String>)]) -> bool {
    constraints.iter().all(|(loader, alternatives)| {
        alternatives.is_empty()
            || provided.version.as_deref().is_some_and(|version| {
                modcompat::satisfies(*loader, alternatives, version) == Some(true)
            })
    })
}

fn dependency_state(
    game_dir: &Path,
    incoming: &[PathBuf],
    loader: Loader,
) -> Result<ModDependencyState> {
    let folder = game_dir.join(ResourceKind::Mod.folder());
    let mut paths = match std::fs::read_dir(&folder) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()).at(&folder))
            .collect::<Result<Vec<_>>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).at(&folder),
    };
    paths.extend_from_slice(incoming);
    paths.sort();
    paths.dedup();
    let mut jars = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !(name.ends_with(".jar") || name.ends_with(".jar.disabled")) {
            continue;
        }
        if !std::fs::metadata(&path).at(&path)?.is_file() {
            continue;
        }
        let descriptors = modcompat::descriptors_of_jar(std::fs::File::open(&path).at(&path)?)?;
        jars.push(DependencyJar {
            path,
            enabled: !name.ends_with(".disabled"),
            descriptors,
        });
    }
    Ok(dependency_state_of(jars, loader))
}

async fn scan_mod_dependencies(
    ctx: &InstallContext,
    incoming: &[PathBuf],
    loader: Loader,
) -> Result<ModDependencyState> {
    let (game_dir, incoming) = (ctx.game_dir.clone(), incoming.to_vec());
    tokio::task::spawn_blocking(move || dependency_state(&game_dir, &incoming, loader))
        .await
        .map_err(|error| Error::Other(error.to_string()))?
}

fn dependency_state_of(jars: Vec<DependencyJar>, loader: Loader) -> ModDependencyState {
    let mut requirements: ModRequirements = BTreeMap::new();
    let mut protected = HashSet::new();
    let mut providers: HashMap<String, Vec<Provided>> = HashMap::new();
    for jar in &jars {
        protected.extend(
            jar.descriptors
                .iter()
                .flat_map(|descriptor| &descriptor.provides)
                .map(|provided| provided.id.to_ascii_lowercase()),
        );
        if !jar.enabled {
            continue;
        }
        let Some(descriptor) = dependency_descriptor(&jar.descriptors, loader) else {
            continue;
        };
        for provided in &descriptor.provides {
            providers
                .entry(provided.id.to_ascii_lowercase())
                .or_default()
                .push(provided.clone());
        }
        for requirement in &descriptor.requires {
            let id = requirement.id.to_ascii_lowercase();
            if !modcompat::PLATFORM_IDS.contains(&id.as_str()) {
                requirements
                    .entry(id)
                    .or_default()
                    .push((descriptor.loader, requirement.versions.clone()));
            }
        }
    }
    let missing = requirements
        .iter()
        .filter_map(|(id, constraints)| {
            let satisfied = providers.get(id).is_some_and(|versions| {
                versions
                    .iter()
                    .any(|provided| provides_requirement(provided, constraints))
            });
            (!satisfied).then(|| id.clone())
        })
        .collect();
    ModDependencyState {
        jars,
        requirements,
        protected,
        missing,
    }
}

fn valid_dependency_candidate(
    descriptors: &[Descriptor],
    state: &ModDependencyState,
    id: &str,
    ctx: &InstallContext,
) -> bool {
    let Some(loader) = ctx.loaders.first().copied() else {
        return false;
    };
    let Some(descriptor) = dependency_descriptor(descriptors, loader) else {
        return false;
    };
    if !descriptor.minecraft.is_empty()
        && !descriptor.minecraft.iter().any(|spec| {
            modcompat::matches_spec(descriptor.loader, spec, &ctx.game_version) == Some(true)
        })
    {
        return false;
    }
    descriptor
        .provides
        .iter()
        .any(|provided| provided.id.eq_ignore_ascii_case(id))
        && descriptor.provides.iter().all(|provided| {
            let provided_id = provided.id.to_ascii_lowercase();
            !state.protected.contains(&provided_id)
                && state
                    .requirements
                    .get(&provided_id)
                    .is_none_or(|constraints| provides_requirement(provided, constraints))
        })
}

fn dependency_url_allowed(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.scheme() == "https" {
        return true;
    }
    // Local HTTP fixtures exercise the real downloader without weakening release builds.
    #[cfg(test)]
    if url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    {
        return true;
    }
    false
}

fn dependency_filename(file: &VersionFile) -> Option<String> {
    let name = sanitize_file_name(&file.filename).ok()?;
    (name == file.filename && !name.starts_with('.') && name.to_ascii_lowercase().ends_with(".jar"))
        .then_some(name)
}

fn dependency_destination_available(
    game_dir: &Path,
    incoming: &[PathBuf],
    name: &str,
) -> Result<bool> {
    let collides = |path: &Path| {
        let existing = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        existing
            .strip_suffix(".disabled")
            .unwrap_or(&existing)
            .eq_ignore_ascii_case(name)
    };
    if incoming.iter().any(|path| collides(path)) {
        return Ok(false);
    }
    let folder = game_dir.join(ResourceKind::Mod.folder());
    match std::fs::read_dir(&folder) {
        Ok(entries) => {
            for entry in entries {
                if collides(&entry.at(&folder)?.path()) {
                    return Ok(false);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).at(&folder),
    }
    Ok(true)
}

fn place_dependency_file(source: &Path, destination: &Path) -> Result<bool> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    match std::fs::hard_link(source, destination) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).at(destination),
    }
}

/// Same-volume staging keeps validated jars outside `mods`, and permits atomic, no-clobber linking.
struct DependencyStaging(PathBuf);

impl DependencyStaging {
    fn new(game_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(game_dir).at(game_dir)?;
        let path = game_dir.join(format!(".mod-dependency-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).at(&path)?;
        Ok(Self(path))
    }
}

impl Drop for DependencyStaging {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            tracing::warn!(
                "cannot remove dependency staging directory {}: {error}",
                self.0.display()
            );
        }
    }
}

fn dependency_aliases(id: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    if id == "fabric"
        || id == "fabric_api"
        || id.starts_with("fabric-")
        || id.starts_with("fabric_")
    {
        aliases.push("fabric-api".to_owned());
    }
    if id == "architectury" {
        aliases.push("architectury-api".to_owned());
    }
    if id == "forgeconfigapiport" {
        aliases.push("forge-config-api-port".to_owned());
    }
    aliases.push(id.replace('_', "-"));
    aliases.push(id.to_owned());
    aliases.retain(|alias| {
        !alias.is_empty()
            && alias
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    });
    aliases.dedup();
    aliases
}

fn is_catalog_not_found(error: &Error) -> bool {
    matches!(error, Error::Http { source, .. } if source.status() == Some(reqwest::StatusCode::NOT_FOUND))
}

/// Installs only absent descriptor dependencies; never updates, replaces or enables a user's mod.
/// `incoming` are inspected together with installed jars but are not copied into the instance.
/// Modrinth is the candidate source; unknown or unsupported dependencies remain mod IDs in the report.
/// Success is determined by a fresh descriptor scan, including transitive requirements.
/// Network, checksum and I/O errors are returned, not converted into a misleading success report.
pub async fn install_missing_mod_dependencies(
    clients: &Clients,
    ctx: &InstallContext,
    incoming: &[PathBuf],
    progress: &ProgressFn,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    let Some(loader) = ctx.loaders.first().copied() else {
        report
            .unresolved_dependencies
            .push("mod-loader (no enabled mod loader)".to_owned());
        return Ok(report);
    };
    let mut parents: Option<Vec<ProjectVersion>> = None;
    let mut attempted = HashSet::new();
    let mut downloads = 0;
    let mut lookups = 0;
    loop {
        let state = scan_mod_dependencies(ctx, incoming, loader).await?;
        if state.missing.is_empty()
            || report.installed.len() >= MAX_LOCAL_DEPENDENCY_INSTALLS
            || downloads >= MAX_LOCAL_DEPENDENCY_DOWNLOADS
            || lookups >= MAX_LOCAL_DEPENDENCY_LOOKUPS
        {
            report.unresolved_dependencies = state.missing;
            return Ok(report);
        }
        // Wrong-version, disabled and other-loader providers are intentionally left to the user.
        let absent: Vec<_> = state
            .missing
            .iter()
            .filter(|id| !state.protected.contains(*id))
            .cloned()
            .collect();
        if absent.is_empty() {
            report.unresolved_dependencies = state.missing;
            return Ok(report);
        }
        if parents.is_none() {
            let mut hashes = Vec::new();
            for jar in state.jars.iter().filter(|jar| jar.enabled) {
                hashes.push(sha1_file(&jar.path).await?);
            }
            let mut known: Vec<_> = clients
                .modrinth
                .versions_by_hashes(&hashes, HashAlgorithm::Sha1)
                .await?
                .into_values()
                .collect();
            known.sort_by(|a, b| {
                a.project_id
                    .cmp(&b.project_id)
                    .then_with(|| a.id.cmp(&b.id))
            });
            parents = Some(known);
        }
        let mut installed = None;
        for id in absent {
            if lookups >= MAX_LOCAL_DEPENDENCY_LOOKUPS {
                break;
            }
            lookups += 1;
            let candidates = local_dependency_candidates(
                clients,
                ctx,
                parents.as_deref().unwrap_or_default(),
                &id,
            )
            .await?;
            for version in candidates {
                if downloads >= MAX_LOCAL_DEPENDENCY_DOWNLOADS {
                    break;
                }
                if !attempted.insert((id.clone(), version.id.clone())) {
                    continue;
                }
                let Some(file) = version.install_file() else {
                    continue;
                };
                let (Some(url), Some(checksum), Some(name)) =
                    (&file.url, checksum_of(file), dependency_filename(file))
                else {
                    continue;
                };
                if !dependency_url_allowed(url)
                    || !dependency_destination_available(&ctx.game_dir, incoming, &name)?
                {
                    continue;
                }
                if packwiz::find_by_project(
                    &packwiz::index_dir(&ctx.game_dir, ResourceKind::Mod),
                    version.platform,
                    &version.project_id,
                )?
                .is_some()
                {
                    continue;
                }
                let project = match clients.modrinth.project(&version.project_id).await {
                    Ok(project) => project,
                    Err(error) if is_catalog_not_found(&error) => continue,
                    Err(error) => return Err(error),
                };
                if project.kind != Some(ResourceKind::Mod) {
                    continue;
                }
                let staging = DependencyStaging::new(&ctx.game_dir)?;
                let path = staging.0.join("candidate.jar");
                progress(ProgressEvent {
                    task: format!("dependency:{id}"),
                    stage: format!("Checking dependency {id}"),
                    done: 0,
                    total: 1,
                });
                downloads += 1;
                download_checked(&clients.http, url, &path, Some(&checksum)).await?;
                let candidate = path.clone();
                let descriptors = tokio::task::spawn_blocking(move || {
                    modcompat::descriptors_of_jar(std::fs::File::open(&candidate).at(&candidate)?)
                })
                .await
                .map_err(|error| Error::Other(error.to_string()))??;
                // Re-read user files after the network work, so newly added providers are protected too.
                let current = scan_mod_dependencies(ctx, incoming, loader).await?;
                if !valid_dependency_candidate(&descriptors, &current, &id, ctx)
                    || !dependency_destination_available(&ctx.game_dir, incoming, &name)?
                {
                    continue;
                }
                let destination = ctx.game_dir.join(ResourceKind::Mod.folder()).join(&name);
                if !place_dependency_file(&path, &destination)? {
                    continue;
                }
                let mut index = PwMod::new(&project, &version, file, false)?;
                index.filename = name.clone();
                packwiz::write_new(
                    &packwiz::index_dir(&ctx.game_dir, ResourceKind::Mod),
                    &format!("dependency-{}", uuid::Uuid::new_v4()),
                    &index,
                )?;
                report.installed.push(InstalledResource {
                    platform: version.platform,
                    project_id: version.project_id.clone(),
                    name: project.title,
                    file_name: name,
                    is_dependency: true,
                });
                progress(ProgressEvent {
                    task: format!("dependency:{id}"),
                    stage: format!("Installed dependency {id}"),
                    done: 1,
                    total: 1,
                });
                installed = Some(version);
                break;
            }
            if installed.is_some() || downloads >= MAX_LOCAL_DEPENDENCY_DOWNLOADS {
                break;
            }
        }
        match installed {
            Some(version) => parents.get_or_insert_default().push(version),
            None => {
                // Do not assume the attempted downloads worked: report the current requirements.
                report.unresolved_dependencies =
                    scan_mod_dependencies(ctx, incoming, loader).await?.missing;
                return Ok(report);
            }
        }
    }
}

async fn local_dependency_candidates(
    clients: &Clients,
    ctx: &InstallContext,
    parents: &[ProjectVersion],
    id: &str,
) -> Result<Vec<ProjectVersion>> {
    // Catalog pins are hints about the project, but must also be respected when selecting its version.
    let mut projects: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for dependency in parents
        .iter()
        .flat_map(|parent| &parent.dependencies)
        .filter(|dependency| dependency.kind == DependencyKind::Required)
        .take(64)
    {
        let project_id = match (&dependency.project_id, &dependency.version_id) {
            (Some(project), _) => project.clone(),
            (None, Some(version)) => match clients.modrinth.version(version).await {
                Ok(version) => version.project_id,
                Err(error) if is_catalog_not_found(&error) => continue,
                Err(error) => return Err(error),
            },
            (None, None) => continue,
        };
        let pins = projects.entry(project_id).or_default();
        if let Some(version) = &dependency.version_id {
            pins.insert(version.clone());
        }
    }
    let pins = projects.clone();
    let mut project_ids: Vec<_> = projects
        .into_keys()
        .take(MAX_LOCAL_DEPENDENCY_PROJECTS / 2)
        .collect();
    project_ids.extend(dependency_aliases(id));
    let search = clients
        .modrinth
        .search(&super::SearchQuery {
            query: id.replace('_', "-"),
            kind: ResourceKind::Mod,
            game_version: Some(ctx.game_version.clone()),
            loaders: ctx.loaders.clone(),
            categories: Vec::new(),
            sort: super::SearchSort::Relevance,
            offset: 0,
            limit: 4,
        })
        .await?;
    project_ids.extend(search.hits.into_iter().map(|hit| hit.project_id));
    let mut seen = HashSet::new();
    // A parent's catalog project is not a dependency, even if a newer release renamed its mod ID.
    let parent_projects: HashSet<_> = parents
        .iter()
        .map(|parent| parent.project_id.as_str())
        .collect();
    project_ids.retain(|project| {
        !parent_projects.contains(project.as_str()) && seen.insert(project.clone())
    });
    let mut candidates = Vec::new();
    for project in project_ids.into_iter().take(MAX_LOCAL_DEPENDENCY_PROJECTS) {
        let lookup = clients
            .modrinth
            .versions(
                &project,
                std::slice::from_ref(&ctx.game_version),
                &ctx.loaders,
            )
            .await;
        let mut versions = match lookup {
            Ok(versions) => versions,
            Err(error) if is_catalog_not_found(&error) => continue,
            Err(error) => return Err(error),
        };
        versions.retain(|version| {
            !parent_projects.contains(version.project_id.as_str())
                && version.game_versions.iter().any(|game| {
                    game == &ctx
                        .game_version
                        .replace(" Pre-Release ", "-pre")
                        .replace(' ', "-")
                })
                && version
                    .loaders
                    .iter()
                    .any(|loader| ctx.loaders.contains(loader))
                && pins.get(&version.project_id).is_none_or(|pins| {
                    pins.is_empty() || (pins.len() == 1 && pins.contains(&version.id))
                })
        });
        versions.sort_by(|a, b| b.published.cmp(&a.published).then_with(|| a.id.cmp(&b.id)));
        candidates.extend(versions.into_iter().take(MAX_LOCAL_DEPENDENCY_VERSIONS));
    }
    candidates.sort_by(|a, b| b.published.cmp(&a.published).then_with(|| a.id.cmp(&b.id)));
    let mut seen = HashSet::new();
    candidates.retain(|version| seen.insert(version.id.clone()));
    Ok(candidates)
}

/// Whether a required dependency of type `dependency` can be installed next to a `parent` resource.
/// Data packs are installed into a world folder, which has no `mods`, `resourcepacks` or `shaderpacks`.
fn can_depend_on(parent: ResourceKind, dependency: ResourceKind) -> bool {
    dependency == parent
        || (parent != ResourceKind::DataPack
            && matches!(dependency, ResourceKind::Mod | ResourceKind::ResourcePack | ResourceKind::ShaderPack))
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

/// Writes a packwiz index for hand-added files that Modrinth (SHA-1) or CurseForge (fingerprint) knows,
/// like Prism does before an update check. Returns how many were identified.
pub async fn identify_local_files(clients: &Clients, game_dir: &std::path::Path, kind: ResourceKind) -> Result<usize> {
    let index_dir = packwiz::index_dir(game_dir, kind);
    let indexed: HashSet<String> = packwiz::list(&index_dir)?.into_iter().map(|entry| entry.data.filename).collect();
    let folder = game_dir.join(kind.folder());
    let mut unknown: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries {
            let entry = entry.at(&folder)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let base = name.strip_suffix(".disabled").unwrap_or(&name).to_owned();
            let is_file = entry.file_type().at(entry.path())?.is_file();
            if is_file && !name.starts_with('.') && !base.ends_with(".pw.toml") && !indexed.contains(&base) {
                unknown.push((base, entry.path()));
            }
        }
    }
    if unknown.is_empty() {
        return Ok(0);
    }

    let mut by_sha1: HashMap<String, usize> = HashMap::new();
    for (index, (_, path)) in unknown.iter().enumerate() {
        by_sha1.insert(sha1_file(path).await?, index);
    }
    let hashes: Vec<String> = by_sha1.keys().cloned().collect();
    let versions = clients.modrinth.versions_by_hashes(&hashes, HashAlgorithm::Sha1).await?;
    let mut found: Vec<(usize, ProjectVersion, String)> = versions
        .into_iter()
        .filter_map(|(hash, version)| Some((*by_sha1.get(&hash)?, version, hash)))
        .collect();
    let mut projects = fetch_projects(clients, Platform::Modrinth, &found).await?;

    if let Some(curseforge) = &clients.curseforge {
        let matched: HashSet<usize> = found.iter().map(|(index, ..)| *index).collect();
        let mut by_fingerprint: HashMap<u32, usize> = HashMap::new();
        for (index, (_, path)) in unknown.iter().enumerate().filter(|(index, _)| !matched.contains(index)) {
            by_fingerprint.insert(murmur2::fingerprint_file(path)?, index);
        }
        let fingerprints: Vec<u32> = by_fingerprint.keys().copied().collect();
        let from_curseforge: Vec<(usize, ProjectVersion, String)> = curseforge
            .versions_by_fingerprints(&fingerprints)
            .await?
            .into_iter()
            .filter_map(|(fingerprint, version)| Some((*by_fingerprint.get(&fingerprint)?, version, String::new())))
            .collect();
        projects.extend(fetch_projects(clients, Platform::CurseForge, &from_curseforge).await?);
        found.extend(from_curseforge);
    }

    let mut identified = 0;
    for (index, version, sha1) in found {
        let (local_name, _) = &unknown[index];
        let Some(project) = projects.get(&(version.platform, version.project_id.clone())) else {
            continue;
        };
        // The file the hash matched; CurseForge versions carry a single file.
        let Some(file) = version
            .files
            .iter()
            .find(|file| sha1.is_empty() || file.hashes.sha1.as_deref() == Some(sha1.as_str()))
        else {
            continue;
        };
        let mut data = PwMod::new(project, &version, file, false)?;
        // The index must name the file as it is on disk, which may differ from the catalog's name.
        data.filename = local_name.clone();
        packwiz::write(&index_dir, &project.slug, &data)?;
        identified += 1;
    }
    Ok(identified)
}

async fn fetch_projects(
    clients: &Clients,
    platform: Platform,
    found: &[(usize, ProjectVersion, String)],
) -> Result<HashMap<(Platform, String), Project>> {
    let mut ids: Vec<String> = found.iter().map(|(_, version, _)| version.project_id.clone()).collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let projects = match (platform, &clients.curseforge) {
        (Platform::Modrinth, _) => clients.modrinth.projects(&ids).await?,
        (Platform::CurseForge, Some(curseforge)) => curseforge.projects(&ids).await?,
        (Platform::CurseForge, None) => Vec::new(),
    };
    Ok(projects.into_iter().map(|project| ((platform, project.id.clone()), project)).collect())
}

/// Drops mis-filed index entries ([`packwiz::remove_misfiled`]), runs [`identify_local_files`], then checks every installed resource of `kind` that has a packwiz index
/// (and no update lock) for a newer version. Modrinth entries are checked in one batch per hash type,
/// CurseForge ones per project. CurseForge entries are skipped when `clients` has no CurseForge client.
pub async fn check_updates(
    clients: &Clients,
    ctx: &InstallContext,
    kind: ResourceKind,
) -> Result<Vec<AvailableUpdate>> {
    packwiz::remove_misfiled(&ctx.game_dir, kind)?;
    identify_local_files(clients, &ctx.game_dir, kind).await?;
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

    fn jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    fn fabric_jar(text: &str) -> Vec<Descriptor> {
        modcompat::descriptors_of_jar(std::io::Cursor::new(jar_bytes(&[(
            "fabric.mod.json",
            text.as_bytes(),
        )])))
        .unwrap()
    }

    fn dependency_jar(descriptors: Vec<Descriptor>, enabled: bool) -> DependencyJar {
        DependencyJar {
            path: PathBuf::new(),
            descriptors,
            enabled,
        }
    }

    #[test]
    fn dependency_catalog_urls_require_https_and_no_credentials() {
        assert!(dependency_url_allowed(
            "https://cdn.modrinth.com/data/project/version/mod.jar"
        ));
        for url in [
            "http://cdn.modrinth.com/mod.jar",
            "file:///tmp/mod.jar",
            "https://user:password@cdn.modrinth.com/mod.jar",
            "not a URL",
        ] {
            assert!(!dependency_url_allowed(url), "{url}");
        }
    }

    #[test]
    fn local_dependencies_satisfy_every_parent_and_alternative_constraint() {
        let state = dependency_state_of(
            vec![
                dependency_jar(
                    fabric_jar(r#"{"id":"parent-a","depends":{"shared":["^1","^3"]}}"#),
                    true,
                ),
                dependency_jar(
                    fabric_jar(r#"{"id":"parent-b","depends":{"shared":">=1.5 <2"}}"#),
                    true,
                ),
            ],
            Loader::Fabric,
        );
        assert_eq!(state.missing, ["shared"]);
        let ctx = ctx(&[Loader::Fabric]);
        for (version, expected) in [
            ("1.4", false),
            ("1.6", true),
            ("2.0", false),
            ("3.0", false),
        ] {
            let candidate = fabric_jar(&format!(
                r#"{{"id":"shared","version":"{version}","depends":{{"minecraft":"1.21.4"}}}}"#
            ));
            assert_eq!(
                valid_dependency_candidate(&candidate, &state, "shared", &ctx),
                expected,
                "{version}"
            );
        }
        let unknown = fabric_jar(r#"{"id":"shared"}"#);
        assert!(!valid_dependency_candidate(
            &unknown, &state, "shared", &ctx
        ));
        let wrong_mc =
            fabric_jar(r#"{"id":"shared","version":"1.6","depends":{"minecraft":"1.20.1"}}"#);
        assert!(!valid_dependency_candidate(
            &wrong_mc, &state, "shared", &ctx
        ));
        let unrelated = fabric_jar(r#"{"id":"not-shared","version":"1.6"}"#);
        assert!(!valid_dependency_candidate(
            &unrelated, &state, "shared", &ctx
        ));
    }

    #[test]
    fn local_dependencies_validate_aliases_and_nested_providers() {
        let nested = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"fabric-module-v0","version":"1.6"}"#,
        )]);
        let api = jar_bytes(&[
            ("fabric.mod.json", br#"{"id":"fabric-api","version":"0.5","provides":["fabric"],"jars":[{"file":"META-INF/jars/module.jar"}]}"#),
            ("META-INF/jars/module.jar", &nested),
        ]);
        let candidate = modcompat::descriptors_of_jar(std::io::Cursor::new(api)).unwrap();
        let parent =
            fabric_jar(r#"{"id":"parent","depends":{"fabric":">=0.5","fabric-module-v0":">=1.5"}}"#);
        let state = dependency_state_of(vec![dependency_jar(parent.clone(), true)], Loader::Fabric);
        assert!(valid_dependency_candidate(
            &candidate,
            &state,
            "fabric-module-v0",
            &ctx(&[Loader::Fabric])
        ));
        let resolved = dependency_state_of(
            vec![
                dependency_jar(parent, true),
                dependency_jar(candidate, true),
            ],
            Loader::Fabric,
        );
        assert!(resolved.missing.is_empty());
        assert!(dependency_aliases("fabric-module-v0").contains(&"fabric-api".to_owned()));
    }

    #[test]
    fn local_dependencies_rescan_transitive_requirements_and_loader_precedence() {
        let parent = fabric_jar(r#"{"id":"parent","depends":{"shared":"*"}}"#);
        let dependency = fabric_jar(r#"{"id":"shared","version":"1","depends":{"transitive":">=2"}}"#);
        let state = dependency_state_of(
            vec![
                dependency_jar(parent, true),
                dependency_jar(dependency, true),
            ],
            Loader::Fabric,
        );
        assert_eq!(state.missing, ["transitive"]);
        let multi = modcompat::descriptors_of_jar(std::io::Cursor::new(jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"id":"fabric-parent","depends":{"fabric-only":"*"}}"#,
            ),
            (
                "quilt.mod.json",
                br#"{"quilt_loader":{"id":"quilt-parent","depends":["quilt-only"]}}"#,
            ),
        ])))
        .unwrap();
        let quilt = dependency_state_of(vec![dependency_jar(multi, true)], Loader::Quilt);
        assert_eq!(quilt.missing, ["quilt-only"]);
    }

    #[test]
    fn local_dependencies_do_not_overwrite_existing_or_disabled_user_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mods");
        std::fs::create_dir(&folder).unwrap();
        let destination = folder.join("dependency.jar");
        std::fs::write(&destination, b"user file").unwrap();
        assert!(!dependency_destination_available(dir.path(), &[], "dependency.jar").unwrap());
        let source = dir.path().join("downloaded.jar");
        std::fs::write(&source, b"catalog file").unwrap();
        // Even a race after the preflight check cannot replace a user's file.
        assert!(!place_dependency_file(&source, &destination).unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), b"user file");
        std::fs::rename(&destination, folder.join("DEPENDENCY.JAR.disabled")).unwrap();
        assert!(!dependency_destination_available(dir.path(), &[], "dependency.jar").unwrap());
        assert!(
            !dependency_destination_available(
                dir.path(),
                &[PathBuf::from("/incoming/other.jar")],
                "other.jar"
            )
            .unwrap()
        );
        let new_destination = folder.join("new.jar");
        assert!(place_dependency_file(&source, &new_destination).unwrap());
        assert_eq!(std::fs::read(new_destination).unwrap(), b"catalog file");
        let staging_path;
        {
            let staging = DependencyStaging::new(dir.path()).unwrap();
            staging_path = staging.0.clone();
            std::fs::write(staging.0.join("candidate.jar"), b"staged").unwrap();
        }
        assert!(!staging_path.exists());
    }

    #[tokio::test]
    async fn local_dependencies_leave_existing_providers_unresolved_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&[Loader::Fabric]);
        ctx.game_dir = dir.path().to_owned();
        let mods = dir.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        let parent = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"parent","depends":{"wrong":">=2","disabled":"*"}}"#,
        )]);
        let wrong = jar_bytes(&[("fabric.mod.json", br#"{"id":"wrong","version":"1"}"#)]);
        let disabled = jar_bytes(&[("fabric.mod.json", br#"{"id":"disabled","version":"1"}"#)]);
        for (name, bytes) in [
            ("parent.jar", &parent),
            ("wrong.jar", &wrong),
            ("disabled.jar.disabled", &disabled),
        ] {
            std::fs::write(mods.join(name), bytes).unwrap();
        }
        let clients = Clients::new(crate::download::http_client(), None).unwrap();
        let report =
            install_missing_mod_dependencies(&clients, &ctx, &[], &crate::progress::no_progress())
                .await
                .unwrap();
        assert!(report.installed.is_empty());
        assert_eq!(report.unresolved_dependencies, ["disabled", "wrong"]);
        assert_eq!(std::fs::read(mods.join("wrong.jar")).unwrap(), wrong);
        assert_eq!(
            std::fs::read(mods.join("disabled.jar.disabled")).unwrap(),
            disabled
        );
    }

    #[tokio::test]
    async fn local_dependencies_without_a_loader_do_not_read_files_or_use_the_network() {
        let clients = Clients::new(crate::download::http_client(), None).unwrap();
        let report = install_missing_mod_dependencies(
            &clients,
            &ctx(&[]),
            &[PathBuf::from("missing.jar")],
            &crate::progress::no_progress(),
        )
        .await
        .unwrap();
        assert!(report.installed.is_empty());
        assert_eq!(
            report.unresolved_dependencies,
            ["mod-loader (no enabled mod loader)"]
        );
    }

    struct CatalogServer {
        clients: Clients,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for CatalogServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl CatalogServer {
        async fn new(handler: impl Fn(&str, &str) -> (u16, Vec<u8>) + Send + 'static) -> Self {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let mut clients = Clients::new(crate::download::http_client(), None).unwrap();
            clients.modrinth =
                super::super::modrinth::Client::with_base_url(clients.http.clone(), base.clone());
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 4096];
                    loop {
                        let length = socket.read(&mut buffer).await.unwrap();
                        if length == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..length]);
                        let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        else {
                            continue;
                        };
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let body_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + body_length {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&request);
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let (status, body) = handler(&base, path);
                    let header = format!(
                        "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                }
            });
            Self { clients, task }
        }
    }

    fn catalog_version(
        base: &str,
        project: &str,
        version: &str,
        bytes: &[u8],
        published: &str,
        dependencies: serde_json::Value,
    ) -> serde_json::Value {
        use sha1::Digest;
        serde_json::json!({
            "id": version, "project_id": project, "name": version, "version_number": "not-the-descriptor-version",
            "version_type": "release", "game_versions": ["1.21.4"], "loaders": ["fabric"],
            "files": [{"url": format!("{base}/download/{version}.jar"), "filename": format!("{version}.jar"),
                "size": bytes.len(), "primary": true, "hashes": {"sha1": hex::encode(sha1::Sha1::digest(bytes))}}],
            "dependencies": dependencies, "date_published": published,
        })
    }

    #[tokio::test]
    async fn local_dependency_downloads_validate_versions_then_recheck_transitives() {
        use serde_json::json;
        use sha1::Digest;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&[Loader::Fabric]);
        ctx.game_dir = dir.path().join("game");
        let parent = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"parent","depends":{"shared":">=1.5 <2"}}"#,
        )]);
        let parent_hash = hex::encode(sha1::Sha1::digest(&parent));
        let incoming = dir.path().join("incoming.jar");
        std::fs::write(&incoming, &parent).unwrap();
        let bad = jar_bytes(&[("fabric.mod.json", br#"{"id":"shared","version":"2.5"}"#)]);
        let good = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"shared","version":"1.6","depends":{"transitive":">=2"}}"#,
        )]);
        let transitive = jar_bytes(&[("fabric.mod.json", br#"{"id":"transitive","version":"2.1"}"#)]);
        let server = CatalogServer::new(move |base, path| {
            let version = |project, id, bytes: &[u8], date, deps| catalog_version(base, project, id, bytes, date, deps);
            let value = match path.split('?').next().unwrap() {
                "/version_files" => json!({parent_hash.clone(): version("parent-project", "parent-version", &parent, "2025", json!([{"project_id":"catalog-dependency","version_id":null,"dependency_type":"required"}]))}),
                "/search" => json!({"hits":[{"project_id":"parent-project","slug":"shared","title":"Parent","description":"test","author":"test","downloads":1,"categories":[]}],"offset":0,"total_hits":1}),
                "/project/shared/version" | "/project/parent-project/version" => json!([version("parent-project", "renamed-parent", &good, "2027", json!([]))]),
                "/project/catalog-dependency/version" => json!([
                    version("catalog-dependency", "new-incompatible", &bad, "2026", json!([])),
                    version("catalog-dependency", "old-compatible", &good, "2025", json!([{"project_id":"transitive-project","version_id":null,"dependency_type":"required"}])),
                ]),
                "/project/transitive-project/version" => json!([version("transitive-project", "transitive-version", &transitive, "2026", json!([]))]),
                "/project/catalog-dependency" | "/project/transitive-project" | "/project/parent-project" => {
                    let project = path.rsplit('/').next().unwrap();
                    json!({"id":project,"slug":project,"title":project,"description":"test","project_type":"mod","client_side":"required","server_side":"required","categories":[]})
                }
                "/download/new-incompatible.jar" => return (200, bad.clone()),
                "/download/old-compatible.jar" | "/download/renamed-parent.jar" => return (200, good.clone()),
                "/download/transitive-version.jar" => return (200, transitive.clone()),
                _ => return (404, b"{}".to_vec()),
            };
            (200, serde_json::to_vec(&value).unwrap())
        }).await;
        let report = install_missing_mod_dependencies(
            &server.clients,
            &ctx,
            &[incoming],
            &crate::progress::no_progress(),
        )
        .await
        .unwrap();
        assert_eq!(report.installed.len(), 2);
        assert!(
            report
                .installed
                .iter()
                .all(|resource| resource.project_id != "parent-project")
        );
        assert!(
            report
                .installed
                .iter()
                .all(|resource| resource.is_dependency)
        );
        assert!(report.unresolved_dependencies.is_empty());
        assert!(ctx.game_dir.join("mods/old-compatible.jar").is_file());
        assert!(ctx.game_dir.join("mods/transitive-version.jar").is_file());
        assert!(!ctx.game_dir.join("mods/new-incompatible.jar").exists());
        assert!(
            !ctx.game_dir.join("mods/incoming.jar").exists(),
            "the parent is not installed by repair"
        );
        assert_eq!(
            packwiz::list(&packwiz::index_dir(&ctx.game_dir, ResourceKind::Mod))
                .unwrap()
                .len(),
            2
        );
        assert!(std::fs::read_dir(&ctx.game_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".mod-dependency-")
        }));
    }

    #[tokio::test]
    async fn local_dependency_unknown_catalog_candidates_are_reported_by_mod_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&[Loader::Fabric]);
        ctx.game_dir = dir.path().join("game");
        let incoming = dir.path().join("incoming.jar");
        std::fs::write(
            &incoming,
            jar_bytes(&[(
                "fabric.mod.json",
                br#"{"id":"parent","depends":{"unknown_id":">=2"}}"#,
            )]),
        )
        .unwrap();
        let server = CatalogServer::new(|_, path| match path.split('?').next().unwrap() {
            "/version_files" => (200, b"{}".to_vec()),
            "/search" => (200, br#"{"hits":[],"offset":0,"total_hits":0}"#.to_vec()),
            _ => (404, b"{}".to_vec()),
        })
        .await;
        let report = install_missing_mod_dependencies(
            &server.clients,
            &ctx,
            &[incoming],
            &crate::progress::no_progress(),
        )
        .await
        .unwrap();
        assert!(report.installed.is_empty());
        assert_eq!(report.unresolved_dependencies, ["unknown_id"]);
    }

    #[tokio::test]
    async fn local_dependency_network_errors_are_not_reported_as_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&[Loader::Fabric]);
        ctx.game_dir = dir.path().join("game");
        let incoming = dir.path().join("incoming.jar");
        std::fs::write(
            &incoming,
            jar_bytes(&[(
                "fabric.mod.json",
                br#"{"id":"parent","depends":{"shared":"*"}}"#,
            )]),
        )
        .unwrap();
        let server = CatalogServer::new(|_, _| (503, b"{}".to_vec())).await;
        let error = install_missing_mod_dependencies(
            &server.clients,
            &ctx,
            &[incoming],
            &crate::progress::no_progress(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Http { .. }));
    }

    fn ctx(loaders: &[Loader]) -> InstallContext {
        InstallContext {
            game_dir: PathBuf::new(),
            game_version: "1.21.4".into(),
            loaders: loaders.to_vec(),
        }
    }

    #[test]
    fn dependencies_keep_their_own_kind_and_never_enter_a_world() {
        use ResourceKind::*;
        assert!(can_depend_on(ResourcePack, Mod), "a pack's mod dependency goes to mods");
        assert!(can_depend_on(Mod, Mod) && can_depend_on(ResourcePack, ResourcePack));
        assert!(!can_depend_on(DataPack, Mod), "a world folder has no mods");
        assert!(!can_depend_on(Mod, Modpack) && !can_depend_on(Mod, World));
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
