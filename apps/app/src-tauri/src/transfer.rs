//! Moving instances and content in and out: export, files dropped onto the window, the lookup of
//! blocked CurseForge files in a folder, and the per-mod update lock.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use k0t1k_core::content::{
    BlockedFile, BlockedScan, HashCache, PackExportReport, PackExportRequest, ResourceKind, add_local_files, collect_blocked,
    export_curseforge, export_mrpack, packwiz,
};
use k0t1k_core::instance::archive::{self, ExportEntry, ExportFilter};
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, CommandResult, blocking, run_unsend};
use crate::events::{progress_fn, tracked};
use crate::state::AppState;

/// File hashes of the folder the user saves blocked mods to, kept between the dialog's polls.
static BLOCKED_HASHES: LazyLock<Mutex<HashCache>> = LazyLock::new(|| Mutex::new(HashCache::new()));

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    /// This launcher's (and Prism-style) instance zip.
    Instance,
    Modrinth,
    CurseForge,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    format: ExportFormat,
    destination: String,
    name: String,
    version: String,
    summary: String,
    /// Game dir paths left out of the archive.
    excluded: Vec<String>,
}

async fn game_dir_of(state: &AppState, instance_id: &str) -> CommandResult<PathBuf> {
    let manager = state.instances.clone();
    let id = instance_id.to_owned();
    blocking(move || Ok(manager.get(&id)?.game_dir(manager.paths()))).await
}

/// The entries of a game dir folder for the export picker (`dir` is empty for the root).
#[tauri::command]
pub async fn list_export_entries(state: State<'_, AppState>, instance_id: String, dir: String) -> CommandResult<Vec<ExportEntry>> {
    let game_dir = game_dir_of(&state, &instance_id).await?;
    blocking(move || archive::list_export_dir(&game_dir, &dir)).await
}

/// Exports the instance. The result is `null` for an instance zip and the numbers of listed and
/// packed files for a Modrinth or CurseForge pack.
#[tauri::command]
pub async fn export_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    request: ExportRequest,
) -> CommandResult<Option<PackExportReport>> {
    let state = state.inner().clone();
    let manager = state.instances.clone();
    let id = instance_id.clone();
    let instance = blocking(move || manager.get(&id)).await?;
    let ExportRequest { format, destination, name, version, summary, excluded } = request;
    let name = name.trim().to_owned();
    let version = version.trim().to_owned();
    if !matches!(format, ExportFormat::Instance) && (name.is_empty() || version.is_empty()) {
        return Err(AppError::new("the pack name and version are required"));
    }

    let task = format!("export:{instance_id}");
    let title = format!("Экспорт: {}", instance.name);
    let progress = progress_fn(&app, &task, &title);
    let destination = PathBuf::from(destination);
    let filter = ExportFilter::new(excluded);
    let (instance_dir, game_dir) = (instance.dir(&state.paths), instance.game_dir(&state.paths));

    tracked(&app, &task, &title, async move {
        if matches!(format, ExportFormat::Instance) {
            blocking(move || archive::export_instance(&instance, &instance_dir, &game_dir, &destination, &filter, &progress)).await?;
            return Ok(None);
        }
        let clients = state.clients.clone();
        let report = run_unsend(move || async move {
            let components: Vec<_> = instance.components.iter().filter(|component| !component.dependency_only).cloned().collect();
            let pack = PackExportRequest {
                game_dir: &game_dir,
                destination: &destination,
                name: &name,
                version: &version,
                summary: Some(summary.trim()),
                components: &components,
                filter: &filter,
            };
            match format {
                ExportFormat::CurseForge => export_curseforge(&clients, &pack, &progress).await,
                _ => export_mrpack(&clients, &pack, &progress).await,
            }
        })
        .await?;
        Ok(Some(report))
    })
    .await
}

/// Copies files dropped onto the window into the instance. With a `kind` they go into that folder;
/// without one each file's type is identified.
#[tauri::command]
pub async fn add_local_files_to_instance(
    state: State<'_, AppState>,
    instance_id: String,
    kind: Option<ResourceKind>,
    paths: Vec<String>,
) -> CommandResult<k0t1k_core::content::AddFilesReport> {
    let game_dir = game_dir_of(&state, &instance_id).await?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    blocking(move || add_local_files(&game_dir, kind, &paths)).await
}

/// Looks for the files of `files` in `folder` (the Downloads folder when `null`) and copies the
/// matches into the instance.
#[tauri::command]
pub async fn scan_blocked_files(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    files: Vec<BlockedFile>,
    folder: Option<String>,
) -> CommandResult<BlockedScan> {
    let folder = match folder {
        Some(folder) => PathBuf::from(folder),
        None => app.path().download_dir().map_err(|error| AppError::new(format!("cannot find the Downloads folder: {error}")))?,
    };
    let game_dir = game_dir_of(&state, &instance_id).await?;
    blocking(move || {
        let mut hashes = BLOCKED_HASHES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        collect_blocked(&game_dir, &files, &folder, &mut hashes)
    })
    .await
}

/// The Downloads folder the blocked-files dialog scans by default.
#[tauri::command]
pub fn get_downloads_dir(app: AppHandle) -> CommandResult<String> {
    let dir = app.path().download_dir().map_err(|error| AppError::new(format!("cannot find the Downloads folder: {error}")))?;
    Ok(dir.display().to_string())
}

/// Locks or unlocks bulk updates of one installed file (packwiz `x-prismlauncher-lock-update`).
#[tauri::command]
pub async fn set_update_lock(
    state: State<'_, AppState>,
    instance_id: String,
    kind: ResourceKind,
    file_name: String,
    locked: bool,
) -> CommandResult<()> {
    let game_dir = game_dir_of(&state, &instance_id).await?;
    blocking(move || {
        let name = file_name.strip_suffix(".disabled").unwrap_or(&file_name);
        packwiz::set_lock_update(&packwiz::index_dir(&game_dir, kind), name, locked)
    })
    .await
}
