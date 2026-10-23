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
mod versions;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = datadir::resolve().and_then(state::AppState::new).map_err(|error| error.to_string())?;
            let webview_dir = state.location.webview_dir();
            std::fs::create_dir_all(&webview_dir)?;
            app.manage(state);
            // Created here rather than in tauri.conf.json to keep WebView2's profile in the data
            // directory instead of `%LOCALAPPDATA%`.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("_k0t1k")
                .inner_size(1200.0, 760.0)
                .min_inner_size(900.0, 560.0)
                .decorations(false)
                .shadow(true)
                .data_directory(webview_dir)
                .build()?;
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
        .run(tauri::generate_context!())
        .expect("failed to run the _k0t1k application");
}
