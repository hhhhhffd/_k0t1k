//! The multiplayer server list of an instance (`servers.dat`) and the server status ping. An instance
//! that shares its list reads and edits `shared/servers.dat`; edits are mirrored into its game dir.

use k0t1k_core::instance::servers::{self, Server, ServerStatus};
use tauri::State;

use crate::content::instance_of;
use crate::error::{CommandResult, blocking};
use crate::state::AppState;
use crate::worlds::ensure_stopped;

#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<Server>> {
    let instance = instance_of(&state, &instance_id).await?;
    let dir = servers::list_dir(&state.paths, &instance);
    blocking(move || servers::list(&dir)).await
}

#[tauri::command]
pub async fn add_server(state: State<'_, AppState>, instance_id: String, server: Server) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let instance = instance_of(&state, &instance_id).await?;
    let paths = state.paths.clone();
    blocking(move || servers::edit_list(&paths, &instance, |dir| servers::add(dir, &server))).await
}

#[tauri::command]
pub async fn update_server(state: State<'_, AppState>, instance_id: String, index: usize, server: Server) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let instance = instance_of(&state, &instance_id).await?;
    let paths = state.paths.clone();
    blocking(move || servers::edit_list(&paths, &instance, |dir| servers::update(dir, index, &server))).await
}

#[tauri::command]
pub async fn remove_server(state: State<'_, AppState>, instance_id: String, index: usize) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let instance = instance_of(&state, &instance_id).await?;
    let paths = state.paths.clone();
    blocking(move || servers::edit_list(&paths, &instance, |dir| servers::remove(dir, index))).await
}

/// Moves the server at `from` to position `to`.
#[tauri::command]
pub async fn move_server(state: State<'_, AppState>, instance_id: String, from: usize, to: usize) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let instance = instance_of(&state, &instance_id).await?;
    let paths = state.paths.clone();
    blocking(move || servers::edit_list(&paths, &instance, |dir| servers::reorder(dir, from, to))).await
}

#[tauri::command]
pub async fn ping_server(address: String) -> CommandResult<ServerStatus> {
    Ok(servers::ping(&address).await?)
}
