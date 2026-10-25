//! Global settings, offline accounts and system information (premium accounts: `accounts`).

use k0t1k_core::i18n::set_language;
use k0t1k_core::instance::SharedFolder;
use k0t1k_core::launch::offline_uuid;
use k0t1k_core::settings::{Account, AccountKind, GlobalSettings, total_memory_mib};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::datadir::{self, DataDirSource};
use crate::error::{AppError, CommandResult, blocking};
use crate::state::AppState;
use crate::tray;

const NAME_LENGTH: std::ops::RangeInclusive<usize> = 3..=16;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    total_ram_mib: Option<u64>,
    os: &'static str,
    app_version: String,
    data_dir: String,
    /// `false` when the data is in the user profile because the executable's folder is read-only
    /// or `data-dir.txt` is unusable.
    portable: bool,
    /// `custom` (data-dir.txt), `portable` (next to the executable) or `fallback` (user profile).
    data_dir_source: DataDirSource,
    exe_dir: String,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<GlobalSettings> {
    let paths = state.paths.clone();
    blocking(move || GlobalSettings::load(&paths)).await
}

/// Accounts, the global playtime and the onboarding record are owned by their own commands and the
/// launch sequence, so a settings form holding an older copy cannot overwrite them.
#[tauri::command]
pub async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: GlobalSettings) -> CommandResult<()> {
    let paths = state.paths.clone();
    let language = settings.ui.language;
    blocking(move || {
        settings.ui.validate()?;
        GlobalSettings::update(&paths, |current| {
            let accounts = std::mem::take(&mut current.accounts);
            let selected_account_id = current.selected_account_id.take();
            let total_secs = current.playtime.total_secs;
            let onboarding = std::mem::take(&mut current.onboarding);
            *current = settings;
            current.onboarding = onboarding;
            current.accounts = accounts;
            current.selected_account_id = selected_account_id;
            current.playtime.total_secs = total_secs;
        })
        .map(|_| ())
    })
    .await?;
    // The tray menu is built from translated labels, so it follows the language change.
    set_language(language);
    tray::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn get_system_info(app: AppHandle, state: State<'_, AppState>) -> SystemInfo {
    SystemInfo {
        total_ram_mib: total_memory_mib(),
        os: std::env::consts::OS,
        app_version: app.package_info().version.to_string(),
        data_dir: state.paths.root().display().to_string(),
        portable: state.location.portable(),
        data_dir_source: state.location.source,
        exe_dir: state.location.exe_dir.display().to_string(),
    }
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    app.opener().open_path(state.paths.root().display().to_string(), None::<&str>)?;
    Ok(())
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum LauncherFolder {
    Instances,
    Java,
    Cache,
    Icons,
    Catpacks,
    Themes,
    Iconsets,
    Logs,
}

/// Opens a folder of the data directory, creating it first so the menu works on a fresh install.
#[tauri::command]
pub async fn open_launcher_folder(app: AppHandle, state: State<'_, AppState>, folder: LauncherFolder) -> CommandResult<()> {
    let dir = match folder {
        LauncherFolder::Instances => state.paths.instances_dir(),
        LauncherFolder::Java => state.paths.java_dir(),
        LauncherFolder::Cache => state.paths.cache_dir(),
        LauncherFolder::Icons => state.paths.icons_dir(),
        LauncherFolder::Catpacks => state.paths.catpacks_dir(),
        LauncherFolder::Themes => state.paths.themes_dir(),
        LauncherFolder::Iconsets => state.paths.iconsets_dir(),
        LauncherFolder::Logs => state.paths.logs_dir(),
    };
    let created = dir.clone();
    blocking(move || std::fs::create_dir_all(&created).map_err(|source| k0t1k_core::Error::Io { path: created, source })).await?;
    app.opener().open_path(dir.display().to_string(), None::<&str>)?;
    Ok(())
}

/// Opens `shared/<folder>`, creating it first so the button works before any instance shares it.
/// The server list is a single file, so for it the `shared` folder itself opens.
#[tauri::command]
pub async fn open_shared_folder(app: AppHandle, state: State<'_, AppState>, folder: SharedFolder) -> CommandResult<()> {
    let dir = match folder {
        SharedFolder::Servers => state.paths.shared_dir(),
        _ => state.paths.shared_dir().join(folder.dir_name()),
    };
    let created = dir.clone();
    blocking(move || std::fs::create_dir_all(&created).map_err(|source| k0t1k_core::Error::Io { path: created, source })).await?;
    app.opener().open_path(dir.display().to_string(), None::<&str>)?;
    Ok(())
}

/// Points the next start at `path`, or back at the default location when `None`. Existing data is
/// neither moved nor deleted. Returns whether a restart is needed: always, the data directory is
/// read once at startup.
#[tauri::command]
pub async fn set_data_dir(state: State<'_, AppState>, path: Option<String>) -> CommandResult<bool> {
    let file = datadir::override_file(&state.location.exe_dir);
    match path.as_deref().map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            let target = path.clone();
            tokio::task::spawn_blocking(move || datadir::check_custom(&target)).await??;
            std::fs::write(&file, path.display().to_string())
                .map_err(|error| AppError::new(format!("cannot write {}: {error}", file.display())))?;
        }
        None => match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::new(format!("cannot remove {}: {error}", file.display()))),
        },
    }
    Ok(true)
}

fn validate_name(name: &str) -> CommandResult<()> {
    let valid = NAME_LENGTH.contains(&name.chars().count()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(AppError::new("a player name must be 3 to 16 characters: letters, digits and underscores"))
    }
}

#[tauri::command]
pub async fn add_offline_account(state: State<'_, AppState>, name: String) -> CommandResult<Account> {
    let name = name.trim().to_owned();
    validate_name(&name)?;
    let account = Account { id: offline_uuid(&name), name, kind: AccountKind::Offline };
    let paths = state.paths.clone();
    blocking(move || {
        let mut added = false;
        GlobalSettings::update(&paths, |settings| {
            added = !settings.accounts.iter().any(|existing| existing.name.eq_ignore_ascii_case(&account.name));
            if added {
                settings.accounts.push(account.clone());
                settings.selected_account_id.get_or_insert_with(|| account.id.clone());
            }
        })?;
        if added {
            Ok(Ok(account))
        } else {
            Ok(Err(AppError::new(format!("the account {} already exists", account.name))))
        }
    })
    .await?
}

/// Removes an account of any kind together with its stored tokens.
#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> CommandResult<()> {
    let paths = state.paths.clone();
    blocking(move || k0t1k_core::auth::remove_account(&paths, &account_id)).await
}

#[tauri::command]
pub async fn select_account(state: State<'_, AppState>, account_id: String) -> CommandResult<()> {
    let paths = state.paths.clone();
    let found = blocking(move || {
        let mut found = false;
        GlobalSettings::update(&paths, |settings| {
            found = settings.accounts.iter().any(|account| account.id == account_id);
            if found {
                settings.selected_account_id = Some(account_id.clone());
            }
        })?;
        Ok(found)
    })
    .await?;
    if found { Ok(()) } else { Err(AppError::new("no such account")) }
}
