//! The in-game overlay agent (`packages/overlay-agent`), built by `build.rs` and embedded in the binary.

use k0t1k_core::component::MINECRAFT_UID;
use k0t1k_core::i18n::{self, tr};
use k0t1k_core::instance::{EffectiveSettings, Instance};
use k0t1k_core::{overlay, tr_fmt};
use tauri::State;

use crate::error::{AppError, CommandResult, blocking};
use crate::events::GameLog;
use crate::state::AppState;

const JAR: &[u8] = include_bytes!(env!("K0T1K_OVERLAY_JAR"));
const VERSION: &str = env!("K0T1K_OVERLAY_VERSION");

/// JVM arguments that load the overlay into a launch; none when it is off or the game predates LWJGL 3.
/// A failure to place the jar is logged and the game starts without the overlay.
pub async fn agent_args(state: &AppState, instance: &Instance, settings: &EffectiveSettings, log: &GameLog) -> Vec<String> {
    if !settings.overlay {
        return Vec::new();
    }
    let minecraft = instance.components.iter().find(|component| component.uid == MINECRAFT_UID).map_or("", |component| component.version.as_str());
    if !overlay::supports(minecraft) {
        log.line("launcher", tr("The overlay needs Minecraft 1.13 or newer; starting without it", "Оверлею нужен Minecraft 1.13 или новее; запуск без него"));
        return Vec::new();
    }
    let dir = state.paths.overlay_dir();
    // this launch is already registered, so one entry means no other game holds an old jar
    let no_other_game = state.running().len() <= 1;
    let target = dir.clone();
    let placed = blocking(move || {
        let jar = overlay::install_jar(&target, VERSION, JAR)?;
        let cleanup = if no_other_game { overlay::remove_old_jars(&target, VERSION).err() } else { None };
        Ok((jar, cleanup))
    })
    .await;
    match placed {
        Ok((jar, cleanup)) => {
            if let Some(error) = cleanup {
                log.line("launcher", &tr_fmt!("Cannot remove an old overlay version: {error}", "Не удалось удалить старую версию оверлея: {error}"));
            }
            overlay::agent_args(&jar, &dir, i18n::language())
        }
        Err(error) => {
            log.line("launcher", &tr_fmt!("Cannot prepare the overlay, starting without it: {error}", "Не удалось подготовить оверлей, запуск без него: {error}"));
            Vec::new()
        }
    }
}

/// Deletes the overlay's chat history. Refused while a game runs, because the overlay writes to it.
#[tauri::command]
pub async fn clear_overlay_history(state: State<'_, AppState>) -> CommandResult<()> {
    if !state.running().is_empty() {
        return Err(AppError::new(tr("stop the game first", "сначала закройте игру")));
    }
    let dir = state.paths.overlay_dir();
    blocking(move || overlay::clear_history(&dir)).await
}
