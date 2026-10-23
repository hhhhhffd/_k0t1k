//! Global settings, accounts and system information.

use k0t1k_core::launch::offline_uuid;
use k0t1k_core::settings::{Account, AccountKind, GlobalSettings, total_memory_mib};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{AppError, CommandResult, blocking};
use crate::state::AppState;

const NAME_LENGTH: std::ops::RangeInclusive<usize> = 3..=16;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    total_ram_mib: Option<u64>,
    os: &'static str,
    app_version: String,
    data_dir: String,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<GlobalSettings> {
    let paths = state.paths.clone();
    blocking(move || GlobalSettings::load(&paths)).await
}

/// Accounts and the global playtime are owned by their own commands and the launch sequence, so a
/// settings form holding an older copy cannot overwrite them.
#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: GlobalSettings) -> CommandResult<()> {
    let paths = state.paths.clone();
    blocking(move || {
        GlobalSettings::update(&paths, |current| {
            let accounts = std::mem::take(&mut current.accounts);
            let selected_account_id = current.selected_account_id.take();
            let total_secs = current.playtime.total_secs;
            *current = settings;
            current.accounts = accounts;
            current.selected_account_id = selected_account_id;
            current.playtime.total_secs = total_secs;
        })
        .map(|_| ())
    })
    .await
}

#[tauri::command]
pub fn get_system_info(app: AppHandle, state: State<'_, AppState>) -> SystemInfo {
    SystemInfo {
        total_ram_mib: total_memory_mib(),
        os: std::env::consts::OS,
        app_version: app.package_info().version.to_string(),
        data_dir: state.paths.root().display().to_string(),
    }
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

#[tauri::command]
pub async fn remove_offline_account(state: State<'_, AppState>, account_id: String) -> CommandResult<()> {
    let paths = state.paths.clone();
    blocking(move || {
        GlobalSettings::update(&paths, |settings| {
            settings.accounts.retain(|account| account.id != account_id);
            if settings.selected_account().is_none() {
                settings.selected_account_id = settings.accounts.first().map(|account| account.id.clone());
            }
        })
        .map(|_| ())
    })
    .await
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
