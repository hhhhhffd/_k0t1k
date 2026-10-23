use k0t1k_core::content::Loader;
use k0t1k_core::meta::{self, MetaClient, VersionEntry};
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

pub fn meta_loader(loader: Loader) -> meta::Loader {
    match loader {
        Loader::Fabric => meta::Loader::Fabric,
        Loader::Quilt => meta::Loader::Quilt,
        Loader::Forge => meta::Loader::Forge,
        Loader::NeoForge => meta::Loader::NeoForge,
    }
}

#[tauri::command]
pub async fn list_minecraft_versions(state: State<'_, AppState>) -> CommandResult<Vec<VersionEntry>> {
    Ok(MetaClient::new(&state.paths, false).minecraft_versions().await?)
}

#[tauri::command]
pub async fn list_loader_versions(
    state: State<'_, AppState>,
    minecraft_version: String,
    loader: Loader,
) -> CommandResult<Vec<VersionEntry>> {
    Ok(MetaClient::new(&state.paths, false).loader_versions(meta_loader(loader), &minecraft_version).await?)
}
