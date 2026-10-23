//! Mods, resource packs, shaders and modpacks: search, install, local files and modpack import.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use k0t1k_core::component::MINECRAFT_UID;
use k0t1k_core::content::packwiz::PwMod;
use k0t1k_core::content::{
    self as core_content, AvailableUpdate, BlockedFile, InstallContext, InstallReport, LocalResource, Loader, Platform,
    ModpackSource, PackState, ProjectVersion, ResourceKind, SearchPage, SearchQuery, SearchSort, inspect_modpack, install_modpack,
    update_modpack as update_installed_modpack,
};
use k0t1k_core::download::{Checksum, download_checked};
use k0t1k_core::error::IoContext;
use k0t1k_core::instance::{Instance, ManagedPack, ManagedPlatform, NewInstance};
use k0t1k_core::settings::MemorySettings;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::{AppError, CommandResult, blocking, run_unsend};
use crate::events::{progress_fn, progress_step, tracked};
use crate::game;
use crate::instances::InstanceView;
use crate::state::AppState;
use crate::versions::meta_loader;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    platform: Platform,
    kind: ResourceKind,
    query: String,
    game_version: Option<String>,
    loader: Option<Loader>,
    offset: u32,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionFilter {
    game_version: Option<String>,
    loader: Option<Loader>,
}

/// Core's metadata carries the icon as PNG bytes; the UI gets it as a data URL.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetadataView {
    id: String,
    name: String,
    version: Option<String>,
    authors: Vec<String>,
    description: Option<String>,
    icon: Option<String>,
    loader: Option<Loader>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalResourceView {
    file_name: String,
    enabled: bool,
    size: u64,
    metadata: Option<ResourceMetadataView>,
    origin: Option<PwMod>,
}

impl From<LocalResource> for LocalResourceView {
    fn from(resource: LocalResource) -> Self {
        Self {
            file_name: resource.file_name,
            enabled: resource.enabled,
            size: resource.size,
            metadata: resource.metadata.map(|metadata| ResourceMetadataView {
                id: metadata.id,
                name: metadata.name,
                version: metadata.version,
                authors: metadata.authors,
                description: metadata.description,
                icon: metadata.icon_png.map(|png| format!("data:image/png;base64,{}", STANDARD.encode(png))),
                loader: metadata.loader,
            }),
            origin: resource.origin,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldView {
    dir_name: String,
    size: u64,
    last_played: Option<DateTime<Utc>>,
}

/// The new instance and the files the author forbids to download; the UI shows their manual links.
#[derive(Serialize)]
pub struct ModpackImport {
    instance: InstanceView,
    blocked: Vec<BlockedFile>,
}

async fn instance_of(state: &AppState, instance_id: &str) -> CommandResult<Instance> {
    let manager = state.instances.clone();
    let id = instance_id.to_owned();
    blocking(move || manager.get(&id)).await
}

fn install_context(state: &AppState, instance: &Instance) -> CommandResult<InstallContext> {
    let game_version = instance
        .components
        .iter()
        .find(|component| component.uid == MINECRAFT_UID)
        .map(|component| component.version.clone())
        .ok_or_else(|| AppError::new("the instance has no Minecraft component"))?;
    let mut loaders = Vec::new();
    for loader in [Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge] {
        if instance.components.iter().any(|component| component.uid == meta_loader(loader).uid()) {
            loaders.push(loader);
            // Quilt runs Fabric mods.
            if loader == Loader::Quilt {
                loaders.push(Loader::Fabric);
            }
        }
    }
    Ok(InstallContext { game_dir: instance.game_dir(&state.paths), game_version, loaders })
}

async fn fetch_version(state: &AppState, platform: Platform, project_id: &str, version_id: &str) -> CommandResult<ProjectVersion> {
    Ok(match platform {
        Platform::Modrinth => state.clients.modrinth.version(version_id).await?,
        Platform::CurseForge => state.curseforge()?.version(project_id, version_id).await?,
    })
}

#[tauri::command]
pub async fn search_content(state: State<'_, AppState>, query: SearchRequest) -> CommandResult<SearchPage> {
    let sort = if query.query.trim().is_empty() { SearchSort::Downloads } else { SearchSort::Relevance };
    let core_query = SearchQuery {
        query: query.query,
        kind: query.kind,
        game_version: query.game_version,
        loaders: query.loader.into_iter().collect(),
        categories: Vec::new(),
        sort,
        offset: query.offset,
        limit: query.limit,
    };
    Ok(match query.platform {
        Platform::Modrinth => state.clients.modrinth.search(&core_query).await?,
        Platform::CurseForge => state.curseforge()?.search(&core_query).await?,
    })
}

#[tauri::command]
pub async fn get_project_versions(
    state: State<'_, AppState>,
    platform: Platform,
    project_id: String,
    filter: VersionFilter,
) -> CommandResult<Vec<ProjectVersion>> {
    let loaders: Vec<Loader> = filter.loader.into_iter().collect();
    Ok(match platform {
        Platform::Modrinth => {
            let game_versions: Vec<String> = filter.game_version.into_iter().collect();
            state.clients.modrinth.versions(&project_id, &game_versions, &loaders).await?
        }
        Platform::CurseForge => state.curseforge()?.versions(&project_id, filter.game_version.as_deref(), &loaders).await?,
    })
}

/// The UI passes the identity of a version it got from `get_project_versions` or `check_updates`;
/// it is fetched again so the download URLs always come from the platform.
#[tauri::command]
pub async fn install_version(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    platform: Platform,
    project_id: String,
    version_id: String,
    kind: ResourceKind,
) -> CommandResult<InstallReport> {
    if matches!(kind, ResourceKind::Modpack | ResourceKind::World) {
        return Err(AppError::new("only mods, resource packs, shaders and data packs can be installed into an instance"));
    }
    let state = state.inner().clone();
    let instance = instance_of(&state, &instance_id).await?;
    let ctx = install_context(&state, &instance)?;
    let version = fetch_version(&state, platform, &project_id, &version_id).await?;

    let task = format!("install:{instance_id}:{}", version.id);
    let title = format!("Установка: {}", version.name);
    let progress = progress_fn(&app, &task, &title);
    tracked(&app, &task, &title, async {
        Ok(core_content::install_version(&state.clients, &ctx, kind, &version, &progress).await?)
    })
    .await
}

#[tauri::command]
pub async fn list_local_resources(state: State<'_, AppState>, instance_id: String, kind: ResourceKind) -> CommandResult<Vec<LocalResourceView>> {
    let instance = instance_of(&state, &instance_id).await?;
    let game_dir = instance.game_dir(&state.paths);
    let resources = blocking(move || core_content::scan_resources(&game_dir, kind)).await?;
    Ok(resources.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn toggle_resource(
    state: State<'_, AppState>,
    instance_id: String,
    kind: ResourceKind,
    file_name: String,
    enabled: bool,
) -> CommandResult<()> {
    let game_dir = instance_of(&state, &instance_id).await?.game_dir(&state.paths);
    blocking(move || core_content::set_enabled(&game_dir, kind, &file_name, enabled).map(|_| ())).await
}

#[tauri::command]
pub async fn delete_resource(state: State<'_, AppState>, instance_id: String, kind: ResourceKind, file_name: String) -> CommandResult<()> {
    let game_dir = instance_of(&state, &instance_id).await?.game_dir(&state.paths);
    blocking(move || core_content::delete_resource(&game_dir, kind, &file_name)).await
}

#[tauri::command]
pub async fn check_updates(state: State<'_, AppState>, instance_id: String, kind: ResourceKind) -> CommandResult<Vec<AvailableUpdate>> {
    let instance = instance_of(&state, &instance_id).await?;
    let ctx = install_context(&state, &instance)?;
    let clients = state.clients.clone();
    run_unsend(move || async move { core_content::check_updates(&clients, &ctx, kind).await }).await
}

fn dir_size(path: &Path) -> k0t1k_core::Result<u64> {
    let mut size = 0;
    for entry in std::fs::read_dir(path).at(path)? {
        let entry = entry.at(path)?;
        let file_type = entry.file_type().at(entry.path())?;
        size += if file_type.is_dir() { dir_size(&entry.path())? } else { entry.metadata().at(entry.path())?.len() };
    }
    Ok(size)
}

fn scan_worlds(game_dir: &Path) -> k0t1k_core::Result<Vec<WorldView>> {
    let saves = game_dir.join(ResourceKind::World.folder());
    let entries = match std::fs::read_dir(&saves) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(&saves),
    };
    let mut worlds = Vec::new();
    for entry in entries {
        let entry = entry.at(&saves)?;
        let path = entry.path();
        if !entry.file_type().at(&path)?.is_dir() {
            continue;
        }
        let level = path.join("level.dat");
        let last_played = match std::fs::metadata(&level) {
            Ok(metadata) => Some(DateTime::from(metadata.modified().at(&level)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).at(&level),
        };
        worlds.push(WorldView { dir_name: entry.file_name().to_string_lossy().into_owned(), size: dir_size(&path)?, last_played });
    }
    worlds.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.dir_name.cmp(&b.dir_name)));
    Ok(worlds)
}

#[tauri::command]
pub async fn list_worlds(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<WorldView>> {
    let game_dir = instance_of(&state, &instance_id).await?.game_dir(&state.paths);
    blocking(move || scan_worlds(&game_dir)).await
}

#[tauri::command]
pub async fn import_modpack_file(app: AppHandle, state: State<'_, AppState>, path: String, name: Option<String>) -> CommandResult<ModpackImport> {
    let state = state.inner().clone();
    let zip_path = PathBuf::from(&path);
    let label = name.clone().unwrap_or_else(|| zip_path.file_stem().map_or_else(|| path.clone(), |stem| stem.to_string_lossy().into_owned()));
    let task = format!("modpack:{path}");
    let title = format!("Импорт модпака: {label}");
    tracked(&app, &task, &title, create_from_pack(&app, &state, zip_path, name, None, &task, &title)).await
}

#[tauri::command]
pub async fn install_modpack_version(
    app: AppHandle,
    state: State<'_, AppState>,
    platform: Platform,
    project_id: String,
    version_id: String,
    name: Option<String>,
) -> CommandResult<ModpackImport> {
    let state = state.inner().clone();
    let version = fetch_version(&state, platform, &project_id, &version_id).await?;
    let project = state.clients.project(platform, &project_id).await?;
    let task = format!("modpack:{platform:?}:{version_id}");
    let title = format!("Установка модпака: {}", name.as_deref().unwrap_or(&project.title));

    let work = async {
        progress_step(&app, &task, &title, "Скачивание пакета", 0, 1);
        let zip_path = download_pack(&state, &version).await?;
        progress_step(&app, &task, &title, "Скачивание пакета", 1, 1);
        let managed_pack = ManagedPack {
            platform: match platform {
                Platform::Modrinth => ManagedPlatform::Modrinth,
                Platform::CurseForge => ManagedPlatform::CurseForge,
            },
            project_id,
            version_id,
            name: project.title.clone(),
            version_name: version.name.clone(),
        };
        create_from_pack(&app, &state, zip_path, name, Some(managed_pack), &task, &title).await
    };
    tracked(&app, &task, &title, work).await
}

/// Updates an instance installed from a modpack to another version of that pack, in place: files the
/// old version brought and the new one dropped are removed, the user's own files stay, and the mods
/// folder is backed up first and restored if the update fails.
#[tauri::command]
pub async fn update_modpack(app: AppHandle, state: State<'_, AppState>, instance_id: String, version_id: String) -> CommandResult<ModpackImport> {
    let state = state.inner().clone();
    if state.is_running(&instance_id) {
        return Err(AppError::new("the instance is running; stop it before updating its modpack"));
    }
    let instance = instance_of(&state, &instance_id).await?;
    let managed = instance.managed_pack.clone().ok_or_else(|| AppError::new("the instance was not installed from a modpack"))?;
    let platform = match managed.platform {
        ManagedPlatform::Modrinth => Platform::Modrinth,
        ManagedPlatform::CurseForge => Platform::CurseForge,
    };
    let version = fetch_version(&state, platform, &managed.project_id, &version_id).await?;
    let task = format!("modpack-update:{instance_id}");
    let title = format!("Обновление модпака: {}", instance.name);

    let work = async {
        progress_step(&app, &task, &title, "Скачивание пакета", 0, 1);
        let zip_path = download_pack(&state, &version).await?;
        progress_step(&app, &task, &title, "Скачивание пакета", 1, 1);
        let inspected = zip_path.clone();
        let manifest = blocking(move || inspect_modpack(&inspected)).await?;
        // Resolving also checks that the new version's Minecraft and loader exist, before any file changes.
        let (components, _) = game::resolve(&state.paths, manifest.components.clone()).await?;

        let (instance_dir, game_dir) = (instance.dir(&state.paths), instance.game_dir(&state.paths));
        let (clients, pack, progress) = (state.clients.clone(), manifest.clone(), progress_fn(&app, &task, &title));
        let report =
            run_unsend(move || async move { update_installed_modpack(&pack, &zip_path, &instance_dir, &game_dir, &clients, &progress).await })
                .await?;

        let manager = state.instances.clone();
        let new_components = manifest.components.clone();
        let instance = blocking(move || {
            manager.update(&instance_id, |instance| {
                instance.components = new_components;
                instance.managed_pack = Some(ManagedPack { version_id, version_name: version.name, ..managed });
            })
        })
        .await?;
        game::spawn_game_install(app.clone(), state.clone(), instance.clone(), components);
        Ok(ModpackImport { instance: instance.into(), blocked: report.blocked })
    };
    tracked(&app, &task, &title, work).await
}

/// Downloads the pack file of `version` into the cache; a file that already matches its hash is reused.
async fn download_pack(state: &AppState, version: &ProjectVersion) -> CommandResult<PathBuf> {
    let file = version.install_file().ok_or_else(|| AppError::new("the modpack version has no files"))?;
    let url = file.url.as_deref().ok_or_else(|| {
        AppError::new(format!(
            "the author disallows third-party downloads of {}; download it manually from {} and import the file",
            file.filename,
            file.download_page.as_deref().unwrap_or("the project page")
        ))
    })?;
    let file_name = Path::new(&file.filename)
        .file_name()
        .ok_or_else(|| AppError::new(format!("{:?} is not a usable file name", file.filename)))?;
    let destination = state.paths.cache_dir().join("modpacks").join(file_name);
    let checksum = match (&file.hashes.sha512, &file.hashes.sha1) {
        (Some(hash), _) => Some(Checksum::Sha512(hash.to_ascii_lowercase())),
        (None, Some(hash)) => Some(Checksum::Sha1(hash.to_ascii_lowercase())),
        (None, None) => None,
    };
    download_checked(&state.http, url, &destination, checksum.as_ref()).await?;
    Ok(destination)
}

/// Creates the instance described by a pack file, fills its game dir and starts installing the game.
/// An instance whose pack content failed to install is removed again.
async fn create_from_pack(
    app: &AppHandle,
    state: &AppState,
    zip_path: PathBuf,
    name: Option<String>,
    managed_pack: Option<ManagedPack>,
    task: &str,
    title: &str,
) -> CommandResult<ModpackImport> {
    let progress = progress_fn(app, task, title);
    let inspected = zip_path.clone();
    let manifest = blocking(move || inspect_modpack(&inspected)).await?;
    // Resolving also checks that the pack's Minecraft and loader versions exist.
    let (components, _) = game::resolve(&state.paths, manifest.components.clone()).await?;

    // An instance zip brings its own notes, icon, settings and the pack it was installed from.
    let from_zip = match &manifest.source {
        ModpackSource::Instance(source) => Some(source.clone()),
        _ => None,
    };
    let new = NewInstance {
        name: name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).unwrap_or_else(|| manifest.name.clone()),
        icon: None,
        group: None,
        components: manifest.components.clone(),
        managed_pack: managed_pack.or_else(|| from_zip.as_ref().and_then(|source| source.managed_pack.clone())),
    };
    let manager = state.instances.clone();
    let recommended_ram = manifest.recommended_ram_mb;
    let instance = blocking(move || {
        let instance = manager.create(new)?;
        if recommended_ram.is_none() && from_zip.is_none() {
            return Ok(instance);
        }
        manager
            .update(&instance.id, |instance| {
                if let Some(max_mib) = recommended_ram {
                    let min_mib = MemorySettings::default().min_mib.min(max_mib);
                    instance.settings.memory = Some(MemorySettings { min_mib, max_mib });
                }
                if let Some(source) = from_zip {
                    instance.notes = source.notes;
                    instance.settings = source.settings;
                    if let Some(icon) = source.icon {
                        instance.icon = icon;
                    }
                }
            })
            .map_err(|error| match manager.delete(&instance.id) {
                Ok(()) => error,
                Err(cleanup) => k0t1k_core::Error::Other(format!("{error}; the partly created instance could not be removed: {cleanup}")),
            })
    })
    .await?;

    let game_dir = instance.game_dir(&state.paths);
    let instance_dir = instance.dir(&state.paths);
    let clients = state.clients.clone();
    let pack = manifest.clone();
    let installed = run_unsend(move || async move {
        let report = install_modpack(&pack, &zip_path, &game_dir, &clients, &progress).await?;
        if report.state != PackState::default() {
            report.state.save(&instance_dir)?;
        }
        Ok(report)
    })
    .await;
    let report = match installed {
        Ok(report) => report,
        Err(error) => {
            let manager = state.instances.clone();
            let id = instance.id.clone();
            let message = match blocking(move || manager.delete(&id)).await {
                Ok(()) => error.to_string(),
                Err(cleanup) => format!("{error}; the partly created instance could not be removed: {cleanup}"),
            };
            return Err(AppError::new(message));
        }
    };

    game::spawn_game_install(app.clone(), state.clone(), instance.clone(), components);
    Ok(ModpackImport { instance: instance.into(), blocked: report.blocked })
}
