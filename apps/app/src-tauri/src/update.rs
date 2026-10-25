//! Launcher self-update. Core finds, downloads and verifies the new executable (`k0t1k_core::update`);
//! this module swaps files and restarts. The launcher is a portable exe, so there is no installer:
//! the running exe is renamed to `<exe>.old` (Windows allows renaming a running image), the verified
//! `<exe>.new` takes its name, and the new process is started with `--wait-for <pid>`, which makes it
//! wait for this process to exit before the single-instance plugin is created. The next start deletes
//! the leftovers.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use k0t1k_core::i18n::tr;
use k0t1k_core::update::{self, UpdateCheck};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{AppError, CommandResult};
use crate::events::{progress_fn, tracked};
use crate::onboarding::{PORTABLE_HERE, WAIT_FOR, spawn_detached};
use crate::state::AppState;

/// The result of the last check, so a window recreated later (the launcher hides to the tray while
/// a game runs) shows the notice without asking GitHub again.
static LAST_CHECK: Mutex<Option<UpdateStatus>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current_version: String,
    #[serde(flatten)]
    check: UpdateCheck,
    /// Why the update cannot be installed from here (dev build, unknown exe path); `None` when it can.
    install_blocked: Option<String>,
}

/// `<exe>.<suffix>` in the exe's own folder.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(exe.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

fn install_blocked() -> Option<String> {
    if !cfg!(windows) {
        return Some(tr("Installing updates is only supported on Windows", "Установка обновлений поддерживается только в Windows").to_owned());
    }
    if cfg!(debug_assertions) {
        return Some(tr("This is a development build, it cannot replace itself", "Это сборка для разработки, она не может заменить себя").to_owned());
    }
    if let Err(error) = std::env::current_exe() {
        tracing::warn!("cannot determine the launcher executable: {error}");
        return Some(tr("The launcher's executable path is unknown", "Не удалось определить путь к exe лаунчера").to_owned());
    }
    None
}

/// Deletes the files a finished or failed update leaves next to the exe. Failures are logged.
pub fn remove_leftovers() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    for suffix in [".old", ".new"] {
        let file = sibling(&exe, suffix);
        match std::fs::remove_file(&file) {
            Ok(()) => tracing::info!("removed {}", file.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!("cannot remove {}: {error}", file.display()),
        }
    }
}

/// Looks for a newer release. Without `force` the result of an earlier check in this run is returned.
/// A failed check is logged here, so a silent startup check may ignore the error.
#[tauri::command]
pub async fn check_for_update(app: AppHandle, state: State<'_, AppState>, force: bool) -> CommandResult<UpdateStatus> {
    if !force && let Some(status) = LAST_CHECK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone() {
        return Ok(status);
    }
    let current_version = app.package_info().version.to_string();
    let check = update::check(&state.http, &current_version).await.inspect_err(|error| tracing::warn!("update check failed: {error}"))?;
    let status = UpdateStatus { current_version, check, install_blocked: install_blocked() };
    *LAST_CHECK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(status.clone());
    Ok(status)
}

fn ensure_no_game_running(state: &AppState) -> CommandResult<()> {
    if state.running().is_empty() {
        Ok(())
    } else {
        Err(AppError::new(tr("Close the running game before updating", "Закройте запущенную игру перед обновлением")))
    }
}

/// Puts `new` in place of `exe`, keeping the old file as `old`. Any failure leaves `exe` as it was.
fn swap_files(exe: &Path, new: &Path, old: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(old) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    std::fs::rename(exe, old)?;
    if let Err(error) = std::fs::rename(new, exe) {
        std::fs::rename(old, exe).map_err(|rollback| {
            std::io::Error::new(rollback.kind(), format!("{error}; and the original executable could not be restored from {}: {rollback}", old.display()))
        })?;
        return Err(error);
    }
    Ok(())
}

/// Downloads the latest release, replaces the exe and restarts. Refused while a game runs.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    if let Some(reason) = install_blocked() {
        return Err(AppError::new(reason));
    }
    ensure_no_game_running(&state)?;
    if INSTALLING.swap(true, Ordering::AcqRel) {
        return Err(AppError::new(tr("The update is already being installed", "Обновление уже устанавливается")));
    }
    let result = install(&app, &state).await;
    // After success the process exits; after a failure another attempt may be made.
    INSTALLING.store(false, Ordering::Release);
    result
}

async fn install(app: &AppHandle, state: &AppState) -> CommandResult<()> {
    let exe = std::env::current_exe()?;
    let current_version = app.package_info().version.to_string();
    let UpdateCheck::Available { release } = update::check(&state.http, &current_version).await? else {
        return Err(AppError::new(tr("No update is available", "Обновление недоступно")));
    };
    let (new, old) = (sibling(&exe, ".new"), sibling(&exe, ".old"));
    let title = tr("Launcher update", "Обновление лаунчера");
    let progress = progress_fn(app, update::TASK, title);
    let swapped = tracked(app, update::TASK, title, async {
        update::download(&state.http, &release, &new, &progress).await?;
        // A game may have been started during the download.
        ensure_no_game_running(state)?;
        let (exe, new, old) = (exe.clone(), new.clone(), old.clone());
        tokio::task::spawn_blocking(move || swap_files(&exe, &new, &old)).await??;
        Ok(())
    })
    .await;
    if let Err(error) = swapped {
        if let Err(cleanup) = std::fs::remove_file(&new)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {cleanup}", new.display());
        }
        return Err(error);
    }

    let mut args = vec![OsString::from(WAIT_FOR), std::process::id().to_string().into()];
    if std::env::args_os().any(|arg| arg == PORTABLE_HERE) {
        args.push(PORTABLE_HERE.into());
    }
    if let Err(error) = spawn_detached(&exe, &args) {
        // The new exe cannot start: put the old one back, so the launcher keeps working.
        std::fs::rename(&old, &exe).map_err(|rollback| {
            AppError::new(format!("{error}; and the original executable could not be restored from {}: {rollback}", old.display()))
        })?;
        return Err(error);
    }
    let app = app.clone();
    tokio::task::spawn_blocking(move || crate::window::exit(&app)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_replaces_the_exe_and_keeps_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let (exe, new, old) = (dir.path().join("a.exe"), dir.path().join("a.exe.new"), dir.path().join("a.exe.old"));
        std::fs::write(&exe, "v1").unwrap();
        std::fs::write(&new, "v2").unwrap();
        std::fs::write(&old, "stale").unwrap();
        swap_files(&exe, &new, &old).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "v2");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "v1");
        assert!(!new.exists());
    }

    #[test]
    fn failed_swap_restores_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let (exe, new, old) = (dir.path().join("a.exe"), dir.path().join("a.exe.new"), dir.path().join("a.exe.old"));
        std::fs::write(&exe, "v1").unwrap();
        // `new` does not exist, so the second rename fails.
        assert!(swap_files(&exe, &new, &old).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "v1");
        assert!(!old.exists());
    }

    #[test]
    fn sibling_appends_to_the_file_name() {
        assert_eq!(sibling(Path::new("/x/_k0t1k.exe"), ".new"), Path::new("/x/_k0t1k.exe.new"));
    }
}
