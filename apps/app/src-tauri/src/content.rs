//! Mods, resource packs, shaders and modpacks: search, install, local files and modpack import.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use k0t1k_core::component::MINECRAFT_UID;
use k0t1k_core::content::pack_url::{PackUrl, parse_pack_url};
use k0t1k_core::content::packwiz::PwMod;
use k0t1k_core::content::{
    self as core_content, AvailableUpdate, BlockedFile, InstallContext, InstallReport, LocalResource, Loader, Platform,
    ModpackSource, PackState, ProjectVersion, ReleaseType, ResourceKind, SearchPage, SearchQuery, SearchSort, inspect_modpack, install_modpack,
    update_modpack as update_installed_modpack,
};
use k0t1k_core::download::{Checksum, download_checked};
use k0t1k_core::instance::archive::read_custom_icon;
use k0t1k_core::instance::icon::custom_file_name;
use k0t1k_core::instance::{Instance, ManagedPack, ManagedPlatform, NewInstance};
use k0t1k_core::launch::tuning::cap_recommended_ram;
use k0t1k_core::settings::{MemorySettings, total_memory_mib};
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

/// The new instance and the files the author forbids to download; the UI shows their manual links.
#[derive(Serialize)]
pub struct ModpackImport {
    pub(crate) instance: InstanceView,
    pub(crate) blocked: Vec<BlockedFile>,
}

pub(crate) async fn instance_of(state: &AppState, instance_id: &str) -> CommandResult<Instance> {
    let manager = state.instances.clone();
    let id = instance_id.to_owned();
    blocking(move || manager.get(&id)).await
}

pub(crate) fn install_context(state: &AppState, instance: &Instance) -> CommandResult<InstallContext> {
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

pub(crate) async fn fetch_version(state: &AppState, platform: Platform, project_id: &str, version_id: &str) -> CommandResult<ProjectVersion> {
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
    install_managed_pack(&app, state.inner(), platform, project_id, version_id, name).await
}

/// Installs one version of a Modrinth or CurseForge modpack as a new instance that remembers its pack.
async fn install_managed_pack(
    app: &AppHandle,
    state: &AppState,
    platform: Platform,
    project_id: String,
    version_id: String,
    name: Option<String>,
) -> CommandResult<ModpackImport> {
    let version = fetch_version(state, platform, &project_id, &version_id).await?;
    let project = state.clients.project(platform, &project_id).await?;
    let task = format!("modpack:{platform:?}:{version_id}");
    let title = format!("Установка модпака: {}", name.as_deref().unwrap_or(&project.title));

    let work = async {
        progress_step(app, &task, &title, "Скачивание пакета", 0, 1);
        let zip_path = download_pack(state, &version).await?;
        progress_step(app, &task, &title, "Скачивание пакета", 1, 1);
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
        create_from_pack(app, state, zip_path, name, Some(managed_pack), &task, &title).await
    };
    tracked(app, &task, &title, work).await
}

/// Installs a modpack from a link: a `curseforge://install` link, a Modrinth or CurseForge modpack page
/// (managed packs, like picking a version in the browser), or a direct link to a pack file.
#[tauri::command]
pub async fn import_modpack_url(app: AppHandle, state: State<'_, AppState>, url: String, name: Option<String>) -> CommandResult<ModpackImport> {
    let state = state.inner();
    match parse_pack_url(&url)? {
        PackUrl::CurseForgeInstall { project_id, file_id } => install_managed_pack(&app, state, Platform::CurseForge, project_id, file_id, name).await,
        PackUrl::Modrinth { project, version } => {
            let project = state.clients.modrinth.project(&project).await?;
            if project.kind != Some(ResourceKind::Modpack) {
                return Err(AppError::new(format!("{} is not a modpack", project.title)));
            }
            let versions = state.clients.modrinth.versions(&project.id, &[], &[]).await?;
            let version = pick_pack_version(&versions, version.as_deref(), &project.title)?;
            install_managed_pack(&app, state, Platform::Modrinth, project.id, version.id.clone(), name).await
        }
        PackUrl::CurseForgePage { slug, file_id } => {
            let curseforge = state.curseforge()?;
            let query = SearchQuery {
                query: slug.clone(),
                kind: ResourceKind::Modpack,
                game_version: None,
                loaders: Vec::new(),
                categories: Vec::new(),
                sort: SearchSort::Relevance,
                offset: 0,
                limit: 20,
            };
            let hit = curseforge
                .search(&query)
                .await?
                .hits
                .into_iter()
                .find(|hit| hit.slug.eq_ignore_ascii_case(&slug))
                .ok_or_else(|| AppError::new(format!("no CurseForge modpack with the address {slug:?} was found")))?;
            let version_id = match file_id {
                Some(file_id) => file_id,
                None => {
                    let versions = curseforge.versions(&hit.project_id, None, &[]).await?;
                    pick_pack_version(&versions, None, &hit.title)?.id.clone()
                }
            };
            install_managed_pack(&app, state, Platform::CurseForge, hit.project_id, version_id, name).await
        }
        PackUrl::File(url) => import_pack_from_link(&app, state, &url, name).await,
    }
}

/// The version asked for by id or version number, else the newest release (the newest of any type if
/// the project has no release).
fn pick_pack_version<'a>(versions: &'a [ProjectVersion], wanted: Option<&str>, title: &str) -> CommandResult<&'a ProjectVersion> {
    if let Some(wanted) = wanted {
        return versions
            .iter()
            .find(|version| version.id == wanted || version.version_number == wanted)
            .ok_or_else(|| AppError::new(format!("{title} has no version {wanted:?}")));
    }
    let newest = |releases_only: bool| {
        versions.iter().filter(|version| !releases_only || version.release_type == ReleaseType::Release).max_by(|a, b| a.published.cmp(&b.published))
    };
    newest(true).or_else(|| newest(false)).ok_or_else(|| AppError::new(format!("{title} has no versions")))
}

/// Downloads the pack file at `url` into a temporary folder of the cache, imports it like a local file
/// and removes the download.
async fn import_pack_from_link(app: &AppHandle, state: &AppState, url: &str, name: Option<String>) -> CommandResult<ModpackImport> {
    let file_name = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.path_segments().and_then(|mut segments| segments.next_back().map(str::to_owned)))
        .map(|segment| segment.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect::<String>())
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| "modpack.zip".to_owned());
    let label = name.clone().unwrap_or_else(|| Path::new(&file_name).file_stem().map_or_else(|| file_name.clone(), |stem| stem.to_string_lossy().into_owned()));
    let task = format!("modpack:{url}");
    let title = format!("Импорт модпака: {label}");

    let work = async {
        let dir = state.paths.cache_dir().join("modpack-links").join(chrono::Utc::now().timestamp_millis().to_string());
        let zip_path = dir.join(&file_name);
        progress_step(app, &task, &title, "Скачивание пакета", 0, 1);
        let imported = match download_checked(&state.http, url, &zip_path, None).await {
            Ok(()) => {
                progress_step(app, &task, &title, "Скачивание пакета", 1, 1);
                create_from_pack(app, state, zip_path, name, None, &task, &title).await
            }
            Err(error) => Err(error.into()),
        };
        if let Err(error) = tokio::fs::remove_dir_all(&dir).await {
            tracing::warn!("the downloaded modpack in {} was not removed: {error}", dir.display());
        }
        imported
    };
    tracked(app, &task, &title, work).await
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
    let icon_zip = zip_path.clone();
    let instance = blocking(move || {
        let instance = manager.create(new)?;
        if recommended_ram.is_none() && from_zip.is_none() {
            return Ok(instance);
        }
        // A custom icon is a file next to instance.json in our zips; it is stored as a new icon of the instance.
        if let Some(icon) = from_zip.as_ref().and_then(|source| source.icon.as_deref())
            && custom_file_name(icon).is_some()
        {
            let stored = read_custom_icon(&icon_zip, icon)
                .and_then(|bytes| bytes.map_or(Ok(()), |bytes| manager.set_custom_icon(&instance.id, &bytes).map(drop)));
            if let Err(error) = stored {
                tracing::warn!("the imported instance icon was skipped: {error}");
            }
        }
        manager
            .update(&instance.id, |instance| {
                if let Some(recommended) = recommended_ram {
                    let max_mib = cap_recommended_ram(recommended, total_memory_mib());
                    let min_mib = MemorySettings::default().min_mib.min(max_mib);
                    instance.settings.memory = Some(MemorySettings { min_mib, max_mib, auto: false });
                }
                if let Some(source) = from_zip {
                    instance.notes = source.notes;
                    instance.settings = source.settings;
                    // A custom icon was stored above; its name in the zip means nothing here.
                    if let Some(icon) = source.icon.filter(|icon| custom_file_name(icon).is_none()) {
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
