//! The game's own settings of an instance (`options.txt`), read and edited by the options tab.

use k0t1k_core::instance::options::{self, GameOption};
use tauri::State;

use crate::error::{CommandResult, blocking};
use crate::state::AppState;
use crate::worlds::{ensure_stopped, game_dir};

/// The options in file order; `None` while the game has not created the file yet.
#[tauri::command]
pub async fn game_options(state: State<'_, AppState>, instance_id: String) -> CommandResult<Option<Vec<GameOption>>> {
    let dir = game_dir(&state, &instance_id).await?;
    blocking(move || options::read(&dir)).await
}

/// Sets the given options; the game would overwrite the change on exit, so a running instance refuses.
#[tauri::command]
pub async fn set_game_options(state: State<'_, AppState>, instance_id: String, changes: Vec<GameOption>) -> CommandResult<()> {
    ensure_stopped(&state, &instance_id)?;
    let dir = game_dir(&state, &instance_id).await?;
    blocking(move || options::write(&dir, &changes)).await
}
