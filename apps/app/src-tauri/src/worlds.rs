//! Worlds of an instance and the data packs inside them. Also home of the helpers that screenshots and
//! servers share: the game dir of an instance and the "game is running" guard.

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Local, Utc};
use k0t1k_core::content::{self as core_content, InstallContext, InstallReport, Platform, ResourceKind};
use k0t1k_core::instance::worlds::{self, GameMode, World};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::content::{LocalResourceView, fetch_version, install_context, instance_of};
use crate::error::{AppError, CommandResult, blocking};
use crate::events::{progress_fn, tracked};
use crate::state::AppState;

/// The game folder of the instance.
pub async fn game_dir(state: &AppState, instance_id: &str) -> CommandResult<PathBuf> {
    Ok(instance_of(state, instance_id).await?.game_dir(&state.paths))
}

/// Saves, the server list and data packs are rewritten by the game; editing them under a running
/// game loses the change or corrupts the file, so such commands refuse.
pub fn ensure_stopped(state: &AppState, instance_id: &str) -> CommandResult<()> {
    if state.is_running(instance_id) {
        return Err(AppError::new("the instance is running; stop the game first"));
    }
    Ok(())
}

/// A world for the UI: the icon is a data URL and the seed a string (a 64-bit seed does not fit a JS number).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldView {
    dir_name: String,
    name: String,
    game_mode: Option<GameMode>,
    hardcore: Option<bool>,
    version_name: Option<String>,
    last_played: Option<DateTime<Utc>>,
    seed: Option<String>,
    icon: Option<String>,
    size: u64,
}

impl From<World> for WorldView {
    fn from(world: World) -> Self {
        Self {
            dir_name: world.dir_name,
            name: world.name,
            game_mode: world.game_mode,
            hardcore: world.hardcore,
            version_name: world.version_name,
            last_played: world.last_played,
            seed: world.seed.map(|seed| seed.to_string()),
            icon: world.icon_png.map(|png| format!("data:image/png;base64,{}", STANDARD.encode(png))),
            size: world.size,
        }
    }
}

#[tauri::command]
pub async fn list_worlds(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<WorldView>> {
    let game_dir = game_dir(&state, &instance_id).await?;
    Ok(blocking(move || worlds::list(&game_dir)).await?.into_iter().map(Into::into).collect())
}

/// Returns the folder name the world has afterwards.
#[tauri::command]
pub async fn rename_world(
    state: State<'_, AppState>,
    instance_id: String,
    dir_name: String,
    name: String,
    rename_folder: bool,
) -> CommandResult<String> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || worlds::rename(&game_dir, &dir_name, &name, rename_folder)).await
}

/// Returns the folder name of the copy.
#[tauri::command]
pub async fn copy_world(state: State<'_, AppState>, instance_id: String, dir_name: String, name: String) -> CommandResult<String> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || worlds::copy(&game_dir, &dir_name, &name)).await
}

#[tauri::command]
pub async fn delete_world(state: State<'_, AppState>, instance_id: String, dir_name: String) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || worlds::delete(&game_dir, &dir_name)).await
}

/// Zips the world into `destination`, or into `<instance>/backups/` when the UI gives none. Returns the
/// archive path.
#[tauri::command]
pub async fn backup_world(
    state: State<'_, AppState>,
    instance_id: String,
    dir_name: String,
    destination: Option<String>,
) -> CommandResult<String> {
    ensure_stopped(&state, &instance_id)?;
    let instance = instance_of(&state, &instance_id).await?;
    let game_dir = instance.game_dir(&state.paths);
    let destination = destination.map(PathBuf::from).unwrap_or_else(|| {
        let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        instance.dir(&state.paths).join("backups").join(format!("{dir_name} {stamp}.zip"))
    });
    let target = destination.clone();
    blocking(move || worlds::backup(&game_dir, &dir_name, &target)).await?;
    Ok(destination.display().to_string())
}

#[tauri::command]
pub async fn open_world_folder(app: AppHandle, state: State<'_, AppState>, instance_id: String, dir_name: String) -> CommandResult<()> {
    let game_dir = game_dir(&state, &instance_id).await?;
    let folder = blocking(move || worlds::world_dir(&game_dir, &dir_name)).await?;
    app.opener().open_path(folder.display().to_string(), None::<&str>)?;
    Ok(())
}

/// Imports a world from a `.zip`; returns the folder name it got.
#[tauri::command]
pub async fn import_world(state: State<'_, AppState>, instance_id: String, path: String) -> CommandResult<String> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || worlds::import_zip(&game_dir, &PathBuf::from(path))).await
}

// ---- data packs of a world: the world folder plays the game dir of the regular resource functions ----

async fn world_folder(state: &AppState, instance_id: &str, world: String) -> CommandResult<PathBuf> {
    let game_dir = game_dir(state, instance_id).await?;
    blocking(move || worlds::world_dir(&game_dir, &world)).await
}

#[tauri::command]
pub async fn list_datapacks(state: State<'_, AppState>, instance_id: String, world: String) -> CommandResult<Vec<LocalResourceView>> {
    let folder = world_folder(&state, &instance_id, world).await?;
    let packs = blocking(move || core_content::scan_resources(&folder, ResourceKind::DataPack)).await?;
    Ok(packs.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn toggle_datapack(
    state: State<'_, AppState>,
    instance_id: String,
    world: String,
    file_name: String,
    enabled: bool,
) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let folder = world_folder(&state, &instance_id, world).await?;
    blocking(move || core_content::set_enabled(&folder, ResourceKind::DataPack, &file_name, enabled).map(|_| ())).await
}

#[tauri::command]
pub async fn delete_datapack(state: State<'_, AppState>, instance_id: String, world: String, file_name: String) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let folder = world_folder(&state, &instance_id, world).await?;
    blocking(move || core_content::delete_resource(&folder, ResourceKind::DataPack, &file_name)).await
}

/// Copies a `.zip` data pack into the world; returns its file name there.
#[tauri::command]
pub async fn add_datapack(state: State<'_, AppState>, instance_id: String, world: String, path: String) -> CommandResult<String> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || worlds::add_datapack(&game_dir, &world, &PathBuf::from(path))).await
}

/// Installs a data pack project version (and its required dependencies) into the world's `datapacks/`.
#[tauri::command]
pub async fn install_datapack(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    world: String,
    platform: Platform,
    project_id: String,
    version_id: String,
) -> CommandResult<InstallReport> {
    ensure_stopped(&state, &instance_id)?;
    let state = state.inner().clone();
    let instance = instance_of(&state, &instance_id).await?;
    let folder = world_folder(&state, &instance_id, world.clone()).await?;
    let ctx = InstallContext { game_dir: folder, ..install_context(&state, &instance)? };
    let version = fetch_version(&state, platform, &project_id, &version_id).await?;

    let task = format!("datapack:{instance_id}:{world}:{}", version.id);
    let title = format!("Установка: {}", version.name);
    let progress = progress_fn(&app, &task, &title);
    tracked(&app, &task, &title, async {
        Ok(core_content::install_version(&state.clients, &ctx, ResourceKind::DataPack, &version, &progress).await?)
    })
    .await
}
