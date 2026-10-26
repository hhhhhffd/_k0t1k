//! Launcher self-update. Core finds, downloads and verifies the new executable (`k0t1k_core::update`);
//! this module swaps files and restarts. The launcher is a portable exe on Windows and an AppImage on
//! Linux, so there is no installer: the running file is renamed to `<file>.old` (Windows allows renaming
//! a running image, Linux keeps the mounted AppImage open), the verified `<file>.new` takes its name,
//! and the new process is started with `--wait-for <pid>`, which makes it wait for this process to exit
//! before the single-instance plugin is created. The next start deletes the leftovers.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use k0t1k_core::i18n::tr;
use k0t1k_core::tr_fmt;
use k0t1k_core::update::{self, UpdateCheck};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{AppError, CommandResult};
use crate::events::{self, progress_fn, tracked};
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
    /// GitHub limited requests or could not be reached; `check` is `unavailable` and the next check may work.
    delayed: bool,
    /// Why the update cannot be installed from here (dev build, unknown exe path); `None` when it can.
    install_blocked: Option<String>,
}

/// `<exe>.<suffix>` in the exe's own folder.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(exe.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// The file an update replaces: the exe on Windows, the AppImage on Linux (the running executable is
/// inside the AppImage's read-only mount or a temporary extraction of it).
fn launcher_file() -> Result<PathBuf, String> {
    if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE").map(PathBuf::from).ok_or_else(|| {
            tr("Updates are installed only into the AppImage, and this launcher was not started from one", "Обновления ставятся только в AppImage, а этот лаунчер запущен не из него").to_owned()
        })
    } else {
        std::env::current_exe().map_err(|error| {
            tracing::warn!("cannot determine the launcher executable: {error}");
            tr("The launcher's executable path is unknown", "Не удалось определить путь к exe лаунчера").to_owned()
        })
    }
}

fn install_blocked() -> Option<String> {
    if !cfg!(any(windows, target_os = "linux")) {
        return Some(tr("Installing updates is only supported on Windows and Linux", "Установка обновлений поддерживается только в Windows и Linux").to_owned());
    }
    if cfg!(debug_assertions) {
        return Some(tr("This is a development build, it cannot replace itself", "Это сборка для разработки, она не может заменить себя").to_owned());
    }
    let exe = match launcher_file() {
        Ok(exe) => exe,
        Err(reason) => return Some(reason),
    };
    // The swap creates and renames files next to the exe, so the folder must be writable (not Program Files, say).
    let probe = sibling(&exe, ".write-test");
    match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&probe).and_then(|_| std::fs::remove_file(&probe)) {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!("the launcher folder is not writable: {error}");
            Some(tr_fmt!("The launcher's folder is not writable: {}", "В папку лаунчера нельзя писать: {}", error))
        }
    }
}

/// Deletes the files a finished or failed update leaves next to the exe. Failures are logged.
pub fn remove_leftovers() {
    let Ok(exe) = launcher_file() else {
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

/// A short message for the UI instead of the technical error, which goes to the log. A rate limit or a
/// network problem is not an alarming failure.
fn failure(error: &k0t1k_core::Error) -> AppError {
    tracing::warn!("update failed: {error}");
    AppError::new(if update::is_transient(error) {
        tr("GitHub is limiting requests right now. We'll check again later.", "GitHub временно ограничил запросы. Проверим позже.")
    } else {
        tr("The update could not be verified or downloaded. Details are in launcher.log.", "Не удалось проверить или скачать обновление. Подробности в launcher.log.")
    })
}

/// Looks for a newer launcher release and, at the same time, installs newer plugins (their results are
/// read with `list_plugins`). Without `force` the result of an earlier check in this run is returned and
/// nothing is asked. A failed check is logged here, so a silent startup check may ignore the error. A
/// rate limit or a network problem is a normal answer (`delayed`) that is not remembered, so the next
/// check asks again.
#[tauri::command]
pub async fn check_for_update(app: AppHandle, state: State<'_, AppState>, force: bool) -> CommandResult<UpdateStatus> {
    if !force && let Some(status) = LAST_CHECK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone() {
        return Ok(status);
    }
    let current_version = app.package_info().version.to_string();
    let install_blocked = install_blocked();
    let (launcher, ()) = tokio::join!(update::check(&current_version), crate::plugins::check_all(state.inner()));
    let check = match launcher {
        Ok(check) => check,
        Err(error) if update::is_transient(&error) => {
            tracing::warn!("update check delayed: {error}");
            return Ok(UpdateStatus { current_version, check: UpdateCheck::Unavailable, delayed: true, install_blocked });
        }
        Err(error) => return Err(failure(&error)),
    };
    let status = UpdateStatus { current_version, check, delayed: false, install_blocked };
    *LAST_CHECK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(status.clone());
    Ok(status)
}

/// Set once an update is about to replace the exe: no game or install may start after it (see
/// `AppState::reserve` and `game::spawn_game_install`, which check it under the locks `ensure_idle` takes).
static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Fails while a game, a game or Java install, or any tracked task (imports, downloads, transfers) is active.
fn ensure_idle(state: &AppState) -> CommandResult<()> {
    let busy = !state.running().is_empty()
        || !state.game_installs().is_empty()
        || state.install_lock.try_lock().is_err()
        || events::active_tasks() > 0;
    if busy {
        Err(AppError::new(tr(
            "Finish or stop running games and downloads before updating",
            "Завершите или остановите запущенные игры и загрузки перед обновлением",
        )))
    } else {
        Ok(())
    }
}

/// What a failed [`swap_files`] left behind.
#[derive(Debug)]
struct SwapError {
    error: std::io::Error,
    /// `false` when the original exe could not be put back and only `<exe>.old` has it.
    restored: bool,
}

/// Puts `new` in place of `exe`, keeping the old file as `old`. Any failure leaves `exe` as it was,
/// or says (`restored`) that it could not.
fn swap_files(exe: &Path, new: &Path, old: &Path) -> Result<(), SwapError> {
    let early = |error| SwapError { error, restored: true };
    match std::fs::remove_file(old) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(early(error)),
        _ => {}
    }
    std::fs::rename(exe, old).map_err(early)?;
    if let Err(error) = std::fs::rename(new, exe) {
        let restored = std::fs::rename(old, exe).is_ok();
        return Err(SwapError { error, restored });
    }
    Ok(())
}

fn manual_recovery(exe: &Path, old: &Path, cause: &dyn std::fmt::Display) -> AppError {
    AppError::new(tr_fmt!(
        "The update failed ({cause}) and the launcher could not be put back. Rename {old} to {exe} to restore it.",
        "Обновление не удалось ({cause}), и лаунчер не удалось вернуть на место. Переименуйте {old} в {exe}, чтобы восстановить его.",
        old = old.display(),
        exe = exe.display()
    ))
}

/// Downloads the release the user saw (`version`), replaces the exe and restarts. Refused while a game
/// runs or anything else is being downloaded or installed.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>, version: String) -> CommandResult<()> {
    if let Some(reason) = install_blocked() {
        return Err(AppError::new(reason));
    }
    ensure_idle(&state)?;
    if INSTALLING.swap(true, Ordering::AcqRel) {
        return Err(AppError::new(tr("The update is already being installed", "Обновление уже устанавливается")));
    }
    let result = install(&app, &state, &version).await;
    // After success the process exits; after a failure another attempt may be made.
    QUITTING.store(false, Ordering::SeqCst);
    INSTALLING.store(false, Ordering::Release);
    result
}

fn remove_new(new: &Path) {
    if let Err(error) = std::fs::remove_file(new)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {error}", new.display());
    }
}

async fn install(app: &AppHandle, state: &AppState, version: &str) -> CommandResult<()> {
    let exe = launcher_file().map_err(AppError::new)?;
    let current_version = app.package_info().version.to_string();
    let client = update::http_client();
    let UpdateCheck::Available { release } = update::check(&current_version).await.map_err(|error| failure(&error))? else {
        return Err(AppError::new(tr("No update is available", "Обновление недоступно")));
    };
    if release.version != version {
        return Err(AppError::new(tr_fmt!(
            "Version {} was released meanwhile, check for updates again",
            "Тем временем вышла версия {}, проверьте обновления ещё раз",
            release.version
        )));
    }
    let (new, old) = (sibling(&exe, ".new"), sibling(&exe, ".old"));
    let title = tr("Launcher update", "Обновление лаунчера");
    let progress = progress_fn(app, update::TASK, title);
    tracked(app, update::TASK, title, async { update::download(&client, &release, &current_version, &new, &progress).await.map_err(|error| failure(&error)) }).await?;
    // The AppImage must stay executable; the download is created with the default mode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }

    // From here nothing new may start; the check after the flag sees everything that started before it.
    QUITTING.store(true, Ordering::SeqCst);
    let swapped = match ensure_idle(state) {
        Ok(()) => {
            let paths = (exe.clone(), new.clone(), old.clone());
            tokio::task::spawn_blocking(move || swap_files(&paths.0, &paths.1, &paths.2)).await?.map_err(|failure| {
                if failure.restored { AppError::from(failure.error) } else { manual_recovery(&exe, &old, &failure.error) }
            })
        }
        Err(error) => Err(error),
    };
    if let Err(error) = swapped {
        // Without a restored exe `.new` is the only other copy, so it is kept.
        if !exe.exists() {
            return Err(error);
        }
        remove_new(&new);
        return Err(error);
    }

    let mut args = vec![OsString::from(WAIT_FOR), std::process::id().to_string().into()];
    if std::env::args_os().any(|arg| arg == PORTABLE_HERE) {
        args.push(PORTABLE_HERE.into());
    }
    if let Err(error) = spawn_detached(&exe, &args) {
        // The new exe cannot start: put the old one back, so the launcher keeps working.
        std::fs::rename(&old, &exe).map_err(|rollback| manual_recovery(&exe, &old, &format!("{error}; {rollback}")))?;
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
