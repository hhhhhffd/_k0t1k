use k0t1k_core::component::{ComponentSpec, MINECRAFT_UID};
use k0t1k_core::content::Loader;
use k0t1k_core::instance::{BrokenInstance, CopyOptions, Instance, InstanceSettings, NewInstance};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
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
    share_resourcepacks: bool,
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
        return Err(AppError::new("the instance is running; stop it before deleting"));
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
        if manager.get(&instance_id)?.share_resourcepacks != settings.share_resourcepacks {
            manager.set_share_resourcepacks(&instance_id, settings.share_resourcepacks)?;
        }
        manager.update(&instance_id, |instance| {
            instance.icon = settings.icon;
            instance.notes = settings.notes;
            instance.settings = settings.settings;
        })
    })
    .await?;
    Ok(instance.into())
}

#[tauri::command]
pub async fn open_instance_folder(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    let manager = state.instances.clone();
    let game_dir = blocking(move || Ok(manager.get(&instance_id)?.game_dir(manager.paths()))).await?;
    app.opener().open_path(game_dir.display().to_string(), None::<&str>)?;
    Ok(())
}
