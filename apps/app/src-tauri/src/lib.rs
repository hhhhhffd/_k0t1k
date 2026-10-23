mod cmdline;
mod content;
mod datadir;
mod error;
mod events;
mod game;
mod instances;
mod java;
mod launch;
mod settings;
mod state;
mod tray;
mod versions;
mod window;

use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. A second launch shows the running launcher; it
        // keeps its state in a named mutex and a hidden window (Windows) or the session bus (Linux).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window::open(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if window.label() == window::MAIN
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                window::request_close(window.app_handle());
            }
        })
        .setup(|app| {
            let state = datadir::resolve().and_then(state::AppState::new).map_err(|error| error.to_string())?;
            let webview_dir = state.location.webview_dir();
            std::fs::create_dir_all(&webview_dir)?;
            app.manage(state);
            window::create(app.handle())?;
            tray::create(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            instances::list_instances,
            instances::create_instance,
            instances::rename_instance,
            instances::delete_instance,
            instances::copy_instance,
            instances::set_instance_group,
            instances::update_instance_settings,
            instances::open_instance_folder,
            launch::launch_instance,
            launch::kill_instance,
            launch::list_run_states,
            launch::get_game_logs,
            versions::list_minecraft_versions,
            versions::list_loader_versions,
            java::detect_java,
            java::list_managed_java,
            settings::get_settings,
            settings::save_settings,
            settings::get_system_info,
            settings::set_data_dir,
            settings::open_data_dir,
            settings::add_offline_account,
            settings::remove_offline_account,
            settings::select_account,
            content::search_content,
            content::get_project_versions,
            content::install_version,
            content::list_local_resources,
            content::toggle_resource,
            content::delete_resource,
            content::check_updates,
            content::list_worlds,
            content::import_modpack_file,
            content::install_modpack_version,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the _k0t1k application")
        .run(|_app, event| {
            // No window left is not a reason to exit: the launcher lives in the tray. Explicit
            // exits (`AppHandle::exit`) carry an exit code and are let through.
            if let RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}
