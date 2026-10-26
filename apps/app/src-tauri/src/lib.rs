mod accounts;
mod clients;
mod cmdline;
mod content;
mod customization;
mod datadir;
mod discord;
mod error;
mod events;
mod ftb;
mod game;
mod instances;
mod java;
mod launch;
mod logging;
mod onboarding;
mod platform;
mod plugins;
mod screenshots;
mod servers;
mod settings;
mod state;
mod transfer;
mod tray;
mod update;
mod versions;
mod window;
mod worlds;

use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    // Before anything else: after a restart the previous process must be gone, or the single-instance
    // plugin below would hand control to it.
    let mut report = None;
    let mut portable_here = false;
    match onboarding::StartupArgs::parse(std::env::args_os().skip(1)) {
        Ok(args) => {
            portable_here = args.portable_here;
            report = onboarding::run_startup_tasks(&args);
        }
        Err(error) => tracing::error!("ignoring the command line: {error}"),
    }
    tauri::Builder::default()
        .manage(onboarding::PendingReport::new(report))
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
        .setup(move |app| {
            let state = datadir::resolve(portable_here).and_then(state::AppState::new).map_err(|error| error.to_string())?;
            // The data directory is final only now; an onboarding root is temporary and removed on exit.
            let onboarding = state.location.source == datadir::DataDirSource::Onboarding;
            if !onboarding {
                logging::init(&state.paths);
                update::remove_leftovers();
                state.instances.remove_leftover_staging();
                k0t1k_core::java::remove_leftover_staging(&state.paths);
            }
            let webview_dir = state.location.webview_dir();
            std::fs::create_dir_all(&webview_dir)?;
            onboarding::sweep_stale_roots(state.paths.root());
            record_created_shortcuts(app.handle(), &state);
            match k0t1k_core::settings::GlobalSettings::load(&state.paths) {
                Ok(settings) => k0t1k_core::i18n::set_language(settings.ui.language),
                Err(error) => tracing::warn!("cannot read the language from the settings: {error}"),
            }
            app.manage(state);
            window::create(app.handle())?;
            if !onboarding && window::HAS_TRAY {
                tray::create(app.handle())?;
            }
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
            instances::change_instance_versions,
            instances::get_instances_dir,
            instances::set_instance_icon,
            instances::reset_instance_icon,
            instances::open_instance_folder,
            launch::launch_instance,
            launch::kill_session,
            launch::kill_instance,
            launch::list_run_states,
            launch::get_game_logs,
            launch::dismiss_crash,
            launch::open_crash_report,
            launch::upload_game_log,
            versions::list_minecraft_versions,
            clients::list_client_versions,
            clients::get_client_terms,
            clients::accept_client_terms,
            versions::list_loader_versions,
            java::detect_java,
            java::list_managed_java,
            java::list_managed_java_usage,
            java::delete_managed_java,
            customization::list_icon_library,
            customization::list_cat_packs,
            customization::list_icon_sets,
            customization::list_theme_files,
            settings::get_settings,
            settings::save_settings,
            settings::get_system_info,
            settings::set_data_dir,
            settings::open_data_dir,
            settings::open_launcher_folder,
            onboarding::get_onboarding,
            onboarding::get_relocation_report,
            onboarding::relocate,
            onboarding::keep_portable_here,
            onboarding::create_shortcuts,
            onboarding::get_uninstall_plan,
            onboarding::uninstall,
            settings::open_shared_folder,
            settings::add_offline_account,
            settings::remove_account,
            accounts::start_microsoft_login,
            accounts::finish_microsoft_login,
            accounts::cancel_microsoft_login,
            accounts::elyby_login,
            settings::select_account,
            content::search_content,
            content::get_project_versions,
            content::install_version,
            content::list_local_resources,
            content::toggle_resource,
            content::delete_resource,
            content::check_updates,
            content::import_modpack_file,
            content::install_modpack_version,
            content::import_modpack_url,
            ftb::ftb_search_packs,
            ftb::ftb_pack_versions,
            ftb::install_ftb_pack,
            worlds::list_worlds,
            worlds::rename_world,
            worlds::copy_world,
            worlds::delete_world,
            worlds::backup_world,
            worlds::open_world_folder,
            worlds::import_world,
            worlds::list_datapacks,
            worlds::toggle_datapack,
            worlds::delete_datapack,
            worlds::add_datapack,
            worlds::install_datapack,
            screenshots::list_screenshots,
            screenshots::open_screenshot,
            screenshots::delete_screenshot,
            screenshots::open_screenshots_folder,
            servers::list_servers,
            servers::add_server,
            servers::update_server,
            servers::remove_server,
            servers::move_server,
            servers::ping_server,
            content::update_modpack,
            transfer::list_export_entries,
            transfer::export_instance,
            transfer::add_local_files_to_instance,
            transfer::scan_blocked_files,
            transfer::get_downloads_dir,
            transfer::set_update_lock,
            plugins::list_plugins,
            plugins::read_plugin_file,
            plugins::ratscan_status,
            plugins::ratscan_store,
            plugins::ratscan_report,
            plugins::read_resource_file,
            plugins::save_resource_file,
            plugins::resourcepack_compat,
            update::check_for_update,
            update::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the _k0t1k application")
        .run(|app, event| match event {
            // No window left is not a reason to exit: the launcher lives in the tray. Explicit
            // exits (`AppHandle::exit`) carry an exit code and are let through.
            RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            RunEvent::Exit => onboarding::remove_own_temporary_root(app),
            _ => {}
        });
}

/// Shortcuts made by the previous process during an install are recorded in the settings of the new
/// data root, which is only known now; a failure to record them is shown with the rest of the report.
fn record_created_shortcuts(app: &tauri::AppHandle, state: &state::AppState) {
    let pending = app.state::<onboarding::PendingReport>();
    if let Some(report) = pending.report().as_mut()
        && let Err(error) = onboarding::record_shortcuts(&state.paths, &report.created_shortcuts)
    {
        report.problems.push(format!("cannot remember the created shortcuts: {error}"));
    }
}
