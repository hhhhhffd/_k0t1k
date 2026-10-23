use k0t1k_core::java::{JavaInstall, detect_java as detect_installed};
use serde::Serialize;
use tauri::State;

use crate::error::CommandResult;
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
