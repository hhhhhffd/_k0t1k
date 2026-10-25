//! Premium accounts: Microsoft device code sign-in and ely.by password sign-in.
//!
//! A Microsoft sign-in takes two commands so the UI can show the code at once: `start_microsoft_login`
//! returns it, `finish_microsoft_login` waits until the user has entered it. `cancel_microsoft_login`
//! stops the wait (closing the dialog).

use k0t1k_core::i18n::tr;
use k0t1k_core::auth::{self, ElybyOutcome, microsoft};
use k0t1k_core::settings::Account;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::watch;

use crate::error::{AppError, CommandResult};
use crate::events;
use crate::state::{AppState, MSA_CLIENT_ID, MicrosoftLogin};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeInfo {
    user_code: String,
    verification_uri: String,
    expires_in: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElybyLoginResult {
    account: Option<Account>,
    /// The account has two-factor auth: call again with the code.
    two_factor_required: bool,
}

fn client_id() -> CommandResult<&'static str> {
    MSA_CLIENT_ID.filter(|id| !id.is_empty()).ok_or_else(|| AppError::new(auth::no_msa_client_id()))
}

#[tauri::command]
pub async fn start_microsoft_login(state: State<'_, AppState>) -> CommandResult<DeviceCodeInfo> {
    let client_id = client_id()?;
    let device = microsoft::request_device_code(&state.http, client_id).await?;
    let info = DeviceCodeInfo {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        expires_in: device.expires_in,
    };
    let (cancel, _) = watch::channel(false);
    let previous = state
        .microsoft_login
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .replace(MicrosoftLogin { device, cancel });
    if let Some(previous) = previous {
        previous.cancel.send_replace(true);
    }
    Ok(info)
}

/// Waits for the user to enter the code, then stores the account. Fails when the sign-in is cancelled,
/// declined or expired.
#[tauri::command]
pub async fn finish_microsoft_login(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Account> {
    let client_id = client_id()?;
    let (device, mut cancelled) = {
        let pending = state.microsoft_login.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = pending.as_ref().ok_or_else(|| AppError::new(tr("Microsoft sign-in has not been started", "вход через Microsoft не начат")))?;
        (pending.device.clone(), pending.cancel.subscribe())
    };
    let on_stage = |stage| events::microsoft_login_stage(&app, stage);
    let outcome = tokio::select! {
        account = auth::complete_microsoft_login(&state.paths, &state.http, client_id, &device, &on_stage) => account.map_err(AppError::from),
        _ = cancelled.wait_for(|cancelled| *cancelled) => Err(AppError::new(tr("Microsoft sign-in was cancelled", "вход через Microsoft отменён"))),
    };
    // A newer sign-in may have replaced this one; only clear our own.
    let mut pending = state.microsoft_login.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if pending.as_ref().is_some_and(|pending| pending.device.device_code == device.device_code) {
        *pending = None;
    }
    outcome
}

#[tauri::command]
pub fn cancel_microsoft_login(state: State<'_, AppState>) {
    let pending = state.microsoft_login.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    if let Some(pending) = pending {
        pending.cancel.send_replace(true);
    }
}

/// The password and the code are only forwarded to ely.by, never stored or logged.
#[tauri::command]
pub async fn elyby_login(
    state: State<'_, AppState>,
    username: String,
    password: String,
    totp: Option<String>,
) -> CommandResult<ElybyLoginResult> {
    let username = username.trim();
    let totp = totp.as_deref().map(str::trim).filter(|code| !code.is_empty());
    if username.is_empty() || password.is_empty() {
        return Err(AppError::new(tr("Enter the ely.by login and password", "введите логин и пароль ely.by")));
    }
    Ok(match auth::elyby_login(&state.paths, &state.http, username, &password, totp).await? {
        ElybyOutcome::Done(account) => ElybyLoginResult { account: Some(account), two_factor_required: false },
        ElybyOutcome::TwoFactorRequired => ElybyLoginResult { account: None, two_factor_required: true },
    })
}
