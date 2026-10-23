//! Steps shared by creating, importing and launching instances.

use k0t1k_core::component::ComponentSpec;
use k0t1k_core::install::install_game;
use k0t1k_core::instance::Instance;
use k0t1k_core::meta::MetaClient;
use k0t1k_core::paths::Paths;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::profile::{LaunchProfile, ResolvedComponent, resolve_components};
use tauri::AppHandle;

use crate::error::CommandResult;
use crate::events::{progress_fn, tracked};
use crate::state::AppState;

/// `os.arch` of a Java matching this machine; the real value is known only once a Java is chosen.
pub fn host_java_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "x86" => "x86",
        other => other,
    }
}

/// Resolves the components through the meta server, falling back to the cached meta when the
/// network fails. The flag tells whether the meta server was used.
pub async fn resolve(paths: &Paths, specs: Vec<ComponentSpec>) -> CommandResult<(Vec<ResolvedComponent>, bool)> {
    match resolve_components(&MetaClient::new(paths, false), specs.clone()).await {
        Ok(components) => Ok((components, true)),
        Err(online_error) => match resolve_components(&MetaClient::new(paths, true), specs).await {
            Ok(components) => Ok((components, false)),
            Err(_) => Err(online_error.into()),
        },
    }
}

/// Downloads libraries and assets of a new instance in the background, so its first launch is fast.
pub fn spawn_game_install(app: AppHandle, state: AppState, instance: Instance, components: Vec<ResolvedComponent>) {
    tauri::async_runtime::spawn(async move {
        let task = format!("game:{}", instance.id);
        let title = format!("Загрузка игры: {}", instance.name);
        let progress = progress_fn(&app, &task, &title);
        let install = async {
            let profile = LaunchProfile::build(&components, RuntimeContext::host(host_java_arch()))?;
            let _installing = state.install_lock.lock().await;
            install_game(&state.http, &state.paths, &profile, &task, &progress).await?;
            Ok(())
        };
        // A failure is reported to the UI by `tracked` as an error event; the next launch retries.
        let _ = tracked(&app, &task, &title, install).await;
    });
}
