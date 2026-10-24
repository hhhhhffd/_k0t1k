//! Feed The Beast modpacks: browse the public FTB catalogue and install a pack version as an instance.

use k0t1k_core::component::{ComponentSpec, MINECRAFT_UID};
use k0t1k_core::content::ftb::{FtbPack, FtbVersion, install_files};
use k0t1k_core::instance::NewInstance;
use tauri::{AppHandle, State};

use crate::content::ModpackImport;
use crate::error::{CommandResult, blocking, run_unsend};
use crate::events::{progress_fn, progress_step, tracked};
use crate::game;
use crate::state::AppState;
use crate::versions::meta_loader;

#[tauri::command]
pub async fn ftb_search_packs(state: State<'_, AppState>, query: String) -> CommandResult<Vec<FtbPack>> {
    Ok(state.clients.ftb.search(&query).await?)
}

#[tauri::command]
pub async fn ftb_pack_versions(state: State<'_, AppState>, pack_id: u64) -> CommandResult<Vec<FtbVersion>> {
    Ok(state.clients.ftb.versions(pack_id).await?)
}

/// Creates an instance with the Minecraft and loader versions of the FTB pack version and downloads its
/// client-side files. Files that cannot be downloaded are returned as blocked. An instance whose files
/// failed to install is removed again.
#[tauri::command]
pub async fn install_ftb_pack(
    app: AppHandle,
    state: State<'_, AppState>,
    pack_id: u64,
    version_id: u64,
    name: Option<String>,
) -> CommandResult<ModpackImport> {
    let state = state.inner().clone();
    let pack = state.clients.ftb.pack(pack_id).await?;
    let task = format!("ftb:{pack_id}:{version_id}");
    let title = format!("Установка модпака: {}", name.as_deref().unwrap_or(&pack.name));

    let work = async {
        progress_step(&app, &task, &title, "Загрузка описания пакета", 0, 1);
        let manifest = state.clients.ftb.manifest(pack_id, version_id).await?;
        let mut specs = vec![ComponentSpec { uid: MINECRAFT_UID.to_owned(), version: manifest.minecraft_version.clone(), dependency_only: false }];
        if let Some(loader) = &manifest.loader {
            specs.push(ComponentSpec { uid: meta_loader(loader.id).uid().to_owned(), version: loader.version.clone(), dependency_only: false });
        }
        // Resolving also checks that the pack's Minecraft and loader versions exist.
        let (components, _) = game::resolve(&state.paths, specs.clone()).await?;

        let new = NewInstance {
            name: name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).unwrap_or_else(|| pack.name.clone()),
            icon: None,
            group: None,
            components: specs,
            managed_pack: None,
        };
        let manager = state.instances.clone();
        let instance = blocking(move || manager.create(new)).await?;

        let game_dir = instance.game_dir(&state.paths);
        let (clients, files, progress, task_id) = (state.clients.clone(), manifest.files, progress_fn(&app, &task, &title), task.clone());
        let installed = run_unsend(move || async move { install_files(&clients, &game_dir, &files, &task_id, &progress).await }).await;
        let blocked = match installed {
            Ok(blocked) => blocked,
            Err(error) => {
                let manager = state.instances.clone();
                let id = instance.id.clone();
                let message = match blocking(move || manager.delete(&id)).await {
                    Ok(()) => error.to_string(),
                    Err(cleanup) => format!("{error}; the partly created instance could not be removed: {cleanup}"),
                };
                return Err(crate::error::AppError::new(message));
            }
        };

        let instance = match set_pack_icon(&state, &instance.id, pack.icon_url.as_deref()).await {
            Ok(Some(with_icon)) => with_icon,
            Ok(None) => instance,
            Err(error) => {
                tracing::warn!("the FTB pack icon was skipped: {error}");
                instance
            }
        };
        game::spawn_game_install(app.clone(), state.clone(), instance.clone(), components);
        Ok(ModpackImport { instance: instance.into(), blocked })
    };
    tracked(&app, &task, &title, work).await
}

/// Stores the pack logo as the instance icon; `None` when the pack has no logo.
async fn set_pack_icon(state: &AppState, instance_id: &str, icon_url: Option<&str>) -> CommandResult<Option<k0t1k_core::instance::Instance>> {
    let Some(url) = icon_url else {
        return Ok(None);
    };
    let bytes = state.http.get(url).send().await?.error_for_status()?.bytes().await?;
    let manager = state.instances.clone();
    let id = instance_id.to_owned();
    Ok(Some(blocking(move || manager.set_custom_icon(&id, &bytes)).await?))
}
