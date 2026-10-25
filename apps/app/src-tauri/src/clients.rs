//! Game clients (LabyMod): which Minecraft versions they support and the terms the user accepts first.

use k0t1k_core::client::ClientKind;
use k0t1k_core::meta::MetaClient;
use k0t1k_core::settings::GlobalSettings;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, CommandResult, blocking};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ClientTerms {
    text: String,
    accepted: bool,
}

/// Minecraft versions the client supports, from its own release manifest.
#[tauri::command]
pub async fn list_client_versions(state: State<'_, AppState>, client: ClientKind) -> CommandResult<Vec<String>> {
    let online = client.supported_versions(&MetaClient::new(&state.paths, false)).await;
    // Offline, the manifest of an earlier run still tells which versions were supported.
    Ok(match online {
        Ok(versions) => versions,
        Err(error) => client.supported_versions(&MetaClient::new(&state.paths, true)).await.map_err(|_| error)?,
    })
}

#[tauri::command]
pub async fn get_client_terms(state: State<'_, AppState>, client: ClientKind) -> CommandResult<ClientTerms> {
    let paths = state.paths.clone();
    let settings = blocking(move || GlobalSettings::load(&paths)).await?;
    let accepted = settings.accepted_client_terms.contains(client.terms_key());
    let text = client.terms(&state.http).await?;
    Ok(ClientTerms { text, accepted })
}

#[tauri::command]
pub async fn accept_client_terms(state: State<'_, AppState>, client: ClientKind) -> CommandResult<()> {
    let paths = state.paths.clone();
    blocking(move || {
        GlobalSettings::update(&paths, |settings| {
            settings.accepted_client_terms.insert(client.terms_key().to_owned());
        })
        .map(|_| ())
    })
    .await
}

/// Refuses to add a client whose terms the user has not accepted.
pub async fn ensure_terms_accepted(state: &AppState, client: ClientKind) -> CommandResult<()> {
    let paths = state.paths.clone();
    let settings = blocking(move || GlobalSettings::load(&paths)).await?;
    if settings.accepted_client_terms.contains(client.terms_key()) {
        Ok(())
    } else {
        Err(AppError::new("the terms of the client have not been accepted"))
    }
}
