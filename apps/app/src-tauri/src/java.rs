use std::collections::HashMap;
use std::path::{Path, PathBuf};

use k0t1k_core::i18n::tr;
use k0t1k_core::java::{JavaInstall, detect_java as detect_installed, required_component};
use k0t1k_core::meta::MetaClient;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::profile::{LaunchProfile, resolve_components};
use k0t1k_core::settings::GlobalSettings;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, CommandResult, blocking, run_unsend};
use crate::game;
use crate::state::AppState;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum JavaSource {
    Managed,
    System,
}

#[derive(Serialize)]
pub struct JavaInstallView {
    #[serde(flatten)]
    install: JavaInstall,
    source: JavaSource,
}

/// Core lists managed runtimes together with the system installs; the UI shows them apart.
async fn installs(state: &AppState, managed: bool) -> Vec<JavaInstallView> {
    let managed_dir = state.paths.java_dir();
    let source = if managed { JavaSource::Managed } else { JavaSource::System };
    detect_installed(&state.paths)
        .await
        .into_iter()
        .filter(|install| install.path.starts_with(&managed_dir) == managed)
        .map(|install| JavaInstallView { install, source })
        .collect()
}

#[tauri::command]
pub async fn detect_java(state: State<'_, AppState>) -> CommandResult<Vec<JavaInstallView>> {
    Ok(installs(&state, false).await)
}

#[tauri::command]
pub async fn list_managed_java(state: State<'_, AppState>) -> CommandResult<Vec<JavaInstallView>> {
    Ok(installs(&state, true).await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedJavaUsage {
    path: PathBuf,
    /// The runtime folder, `java/<component>`.
    dir: PathBuf,
    major: u32,
    /// Instances that would start with this runtime under the current settings.
    used_by: Vec<String>,
    /// A launched instance that uses it is running or starting.
    running: bool,
}

/// The `java/<component>` folder a managed Java executable belongs to.
fn runtime_dir(java_dir: &Path, java: &Path) -> Option<PathBuf> {
    let component = java.strip_prefix(java_dir).ok()?.components().next()?;
    Some(java_dir.join(component))
}

/// Components that instances with automatic Java selection would use, from cached meta only.
/// An instance whose requirements cannot be determined is left out.
async fn auto_components(paths: &k0t1k_core::paths::Paths, wanted: Vec<(String, Vec<k0t1k_core::component::ComponentSpec>)>) -> CommandResult<HashMap<String, String>> {
    let paths = paths.clone();
    run_unsend(move || async move {
        let meta = MetaClient::new(&paths, true);
        let mut components = HashMap::new();
        for (id, specs) in wanted {
            let profile = match resolve_components(&meta, specs).await {
                Ok(resolved) => LaunchProfile::build(&resolved, RuntimeContext::host(game::host_java_arch())),
                Err(error) => Err(error),
            };
            match profile {
                Ok(profile) => {
                    if let Some(component) = required_component(&profile.compatible_java_majors, profile.compatible_java_name.as_deref()) {
                        components.insert(id, component.to_owned());
                    }
                }
                Err(error) => tracing::debug!("cannot tell which Java instance {id} needs: {error}"),
            }
        }
        Ok(components)
    })
    .await
}

#[tauri::command]
pub async fn list_managed_java_usage(state: State<'_, AppState>) -> CommandResult<Vec<ManagedJavaUsage>> {
    let java_dir = state.paths.java_dir();
    let installs = installs(&state, true).await;
    let paths = state.paths.clone();
    let manager = state.instances.clone();
    let (instances, global) = blocking(move || Ok((manager.list()?.instances, GlobalSettings::load(&paths)?))).await?;

    let mut by_path = Vec::new();
    let mut automatic = Vec::new();
    for instance in instances {
        let java = instance.settings.resolve(&global).java;
        match java.path {
            Some(path) => by_path.push((instance.id, path)),
            None if java.auto_select => automatic.push((instance.id, instance.components)),
            None => {}
        }
    }
    let components = auto_components(&state.paths, automatic).await?;
    let running: Vec<String> = state.running().values().map(|entry| entry.instance_id.clone()).collect();

    Ok(installs
        .into_iter()
        .filter_map(|view| {
            let dir = runtime_dir(&java_dir, &view.install.path)?;
            let name = dir.file_name()?.to_string_lossy().into_owned();
            let mut used_by: Vec<String> = by_path
                .iter()
                .filter(|(_, path)| path.starts_with(&dir))
                .map(|(id, _)| id.clone())
                .chain(components.iter().filter(|(_, component)| **component == name).map(|(id, _)| id.clone()))
                .collect();
            used_by.sort();
            used_by.dedup();
            let running = used_by.iter().any(|id| running.contains(id));
            Some(ManagedJavaUsage { path: view.install.path, dir, major: view.install.major, used_by, running })
        })
        .collect())
}

/// Deletes `java/<component>`. The Java of a running game is not recorded, so nothing is deleted
/// while any game is running or starting.
#[tauri::command]
pub async fn delete_managed_java(state: State<'_, AppState>, dir: String) -> CommandResult<()> {
    let java_dir = state.paths.java_dir();
    let dir = PathBuf::from(dir);
    let _installing = state.install_lock.lock().await;
    if !state.running().is_empty() {
        return Err(AppError::new(tr("Java cannot be removed while a game is running", "Нельзя удалить Java, пока запущена игра")));
    }
    blocking(move || {
        let canonical_java_dir = java_dir.canonicalize().map_err(|source| k0t1k_core::Error::Io { path: java_dir.clone(), source })?;
        let canonical = dir.canonicalize().map_err(|source| k0t1k_core::Error::Io { path: dir.clone(), source })?;
        // Only the runtime folders themselves, never the staging folder or anything outside `java/`.
        let is_runtime = canonical.parent() == Some(canonical_java_dir.as_path()) && canonical.is_dir() && canonical.file_name().is_some_and(|name| !name.to_string_lossy().starts_with('.'));
        if !is_runtime {
            return Err(k0t1k_core::Error::Other(format!("{} is not a managed Java runtime", dir.display())));
        }
        tracing::info!("deleting the managed Java runtime {}", canonical.display());
        std::fs::remove_dir_all(&canonical).map_err(|source| k0t1k_core::Error::Io { path: canonical, source })
    })
    .await
}
