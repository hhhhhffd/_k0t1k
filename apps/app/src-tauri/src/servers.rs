//! The multiplayer server list of an instance (`servers.dat`) and the server status ping.

use k0t1k_core::instance::servers::{self, Server, ServerStatus};
use tauri::State;

use crate::error::{CommandResult, blocking};
use crate::state::AppState;
use crate::worlds::{ensure_stopped, game_dir};

#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<Server>> {
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || servers::list(&game_dir)).await
}

#[tauri::command]
pub async fn add_server(state: State<'_, AppState>, instance_id: String, server: Server) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || servers::add(&game_dir, &server)).await
}

#[tauri::command]
pub async fn update_server(state: State<'_, AppState>, instance_id: String, index: usize, server: Server) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || servers::update(&game_dir, index, &server)).await
}

#[tauri::command]
pub async fn remove_server(state: State<'_, AppState>, instance_id: String, index: usize) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || servers::remove(&game_dir, index)).await
}

/// Moves the server at `from` to position `to`.
#[tauri::command]
pub async fn move_server(state: State<'_, AppState>, instance_id: String, from: usize, to: usize) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let game_dir = game_dir(&state, &instance_id).await?;
    blocking(move || servers::reorder(&game_dir, from, to)).await
}

#[tauri::command]
pub async fn ping_server(address: String) -> CommandResult<ServerStatus> {
    Ok(servers::ping(&address).await?)
}
