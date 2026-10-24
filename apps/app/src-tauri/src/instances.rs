use k0t1k_core::component::{ComponentSpec, MINECRAFT_UID};
use k0t1k_core::content::Loader;
use std::collections::BTreeSet;

use k0t1k_core::instance::icon::read_image;
use k0t1k_core::instance::{BrokenInstance, CopyOptions, Instance, InstanceSettings, NewInstance, SharedFolder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, CommandResult, blocking};
use crate::game;
use crate::state::AppState;
use crate::versions::meta_loader;

/// Core keeps the id out of `instance.json` (it is the directory name), so the UI gets it beside the rest.
#[derive(Serialize)]
pub struct InstanceView {
    #[serde(flatten)]
    instance: Instance,
    id: String,
}

impl From<Instance> for InstanceView {
    fn from(instance: Instance) -> Self {
        Self { id: instance.id.clone(), instance }
    }
}

#[derive(Serialize)]
pub struct InstanceListing {
    instances: Vec<InstanceView>,
    errors: Vec<BrokenInstance>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstanceRequest {
    name: String,
    group: Option<String>,
    icon: String,
    minecraft_version: String,
    loader: Option<LoaderChoice>,
}

#[derive(Deserialize)]
pub struct LoaderChoice {
    id: Loader,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSettingsUpdate {
    icon: String,
    notes: String,
    shared_folders: BTreeSet<SharedFolder>,
    #[serde(default)]
    join_server: Option<String>,
    settings: InstanceSettings,
}

fn spec(uid: &str, version: String) -> ComponentSpec {
    ComponentSpec { uid: uid.to_owned(), version, dependency_only: false }
}

#[tauri::command]
pub async fn list_instances(state: State<'_, AppState>) -> CommandResult<InstanceListing> {
    let manager = state.instances.clone();
    let list = blocking(move || manager.list()).await?;
    Ok(InstanceListing { instances: list.instances.into_iter().map(Into::into).collect(), errors: list.errors })
}

#[tauri::command]
pub async fn create_instance(app: AppHandle, state: State<'_, AppState>, request: CreateInstanceRequest) -> CommandResult<InstanceView> {
    let name = request.name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::new("the instance name is empty"));
    }
    let mut specs = vec![spec(MINECRAFT_UID, request.minecraft_version)];
    if let Some(loader) = request.loader {
        specs.push(spec(meta_loader(loader.id).uid(), loader.version));
    }
    // Resolving also checks that every chosen version exists and fits together.
    let (components, _) = game::resolve(&state.paths, specs.clone()).await?;

    let manager = state.instances.clone();
    let new = NewInstance { name, icon: Some(request.icon), group: request.group, components: specs, managed_pack: None };
    let instance = blocking(move || manager.create(new)).await?;
    game::spawn_game_install(app, state.inner().clone(), instance.clone(), components);
    Ok(instance.into())
}

#[tauri::command]
pub async fn rename_instance(state: State<'_, AppState>, instance_id: String, name: String) -> CommandResult<InstanceView> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::new("the instance name is empty"));
    }
    let manager = state.instances.clone();
    Ok(blocking(move || manager.rename(&instance_id, &name)).await?.into())
}

#[tauri::command]
pub async fn delete_instance(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    if state.is_running(&instance_id) {
        return Err(AppError::new("the instance is running; stop all its sessions before deleting"));
    }
    game::cancel_game_install(&app, &state, &instance_id);
    let manager = state.instances.clone();
    blocking(move || manager.delete(&instance_id)).await
}

#[tauri::command]
pub async fn copy_instance(state: State<'_, AppState>, instance_id: String, name: String) -> CommandResult<InstanceView> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::new("the instance name is empty"));
    }
    let manager = state.instances.clone();
    Ok(blocking(move || manager.copy(&instance_id, &name, &CopyOptions::default())).await?.into())
}

#[tauri::command]
pub async fn set_instance_group(state: State<'_, AppState>, instance_id: String, group: Option<String>) -> CommandResult<InstanceView> {
    let manager = state.instances.clone();
    Ok(blocking(move || manager.set_group(&instance_id, group)).await?.into())
}

#[tauri::command]
pub async fn update_instance_settings(
    state: State<'_, AppState>,
    instance_id: String,
    settings: InstanceSettingsUpdate,
) -> CommandResult<InstanceView> {
    let manager = state.instances.clone();
    let instance = blocking(move || {
        manager.set_shared_folders(&instance_id, settings.shared_folders)?;
        manager.update(&instance_id, |instance| {
            instance.icon = settings.icon;
            instance.notes = settings.notes;
            instance.join_server = settings.join_server.map(|server| server.trim().to_owned()).filter(|server| !server.is_empty());
            instance.settings = settings.settings;
        })
    })
    .await?;
    Ok(instance.into())
}

/// Replaces the instance's Minecraft version and mod loader (or removes the loader) and downloads the new
/// game files in the background. Components that are neither of these stay; dependency-only ones are
/// dropped because resolving adds them again in the versions the new set needs.
#[tauri::command]
pub async fn change_instance_versions(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    minecraft_version: String,
    loader: Option<LoaderChoice>,
) -> CommandResult<InstanceView> {
    if state.is_running(&instance_id) {
        return Err(AppError::new("the instance is running; stop it before changing its versions"));
    }
    let manager = state.instances.clone();
    let current = blocking({
        let (manager, id) = (manager.clone(), instance_id.clone());
        move || manager.get(&id)
    })
    .await?;
    let loader_uids = [Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge].map(|l| meta_loader(l).uid());
    let mut specs = vec![spec(MINECRAFT_UID, minecraft_version)];
    if let Some(loader) = loader {
        specs.push(spec(meta_loader(loader.id).uid(), loader.version));
    }
    specs.extend(
        current
            .components
            .into_iter()
            .filter(|c| !c.dependency_only && c.uid != MINECRAFT_UID && !loader_uids.contains(&c.uid.as_str())),
    );
    // Resolving also checks that every chosen version exists and fits together.
    let (components, _) = game::resolve(&state.paths, specs.clone()).await?;

    let instance = blocking(move || manager.update(&instance_id, |instance| instance.components = specs)).await?;
    game::cancel_game_install(&app, &state, &instance.id);
    game::spawn_game_install(app, state.inner().clone(), instance.clone(), components);
    Ok(instance.into())
}

/// Custom icons are files in the instance folders and are shown through the asset protocol, whose scope is
/// empty in config. The UI asks for the folder once at startup; it is the only place the webview may read.
#[tauri::command]
pub fn get_instances_dir(app: AppHandle, state: State<'_, AppState>) -> CommandResult<String> {
    let dir = state.paths.instances_dir();
    app.asset_protocol_scope().allow_directory(&dir, true)?;
    Ok(dir.display().to_string())
}

#[tauri::command]
pub async fn set_instance_icon(state: State<'_, AppState>, instance_id: String, path: String) -> CommandResult<InstanceView> {
    let manager = state.instances.clone();
    let instance = blocking(move || manager.set_custom_icon(&instance_id, &read_image(path.as_ref())?)).await?;
    Ok(instance.into())
}

#[tauri::command]
pub async fn reset_instance_icon(state: State<'_, AppState>, instance_id: String) -> CommandResult<InstanceView> {
    let manager = state.instances.clone();
    Ok(blocking(move || manager.reset_icon(&instance_id)).await?.into())
}

#[tauri::command]
pub async fn open_instance_folder(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    let manager = state.instances.clone();
    let game_dir = blocking(move || Ok(manager.get(&instance_id)?.game_dir(manager.paths()))).await?;
    app.opener().open_path(game_dir.display().to_string(), None::<&str>)?;
    Ok(())
}
