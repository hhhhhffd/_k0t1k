//! The tray icon. It exists for the whole run, so the launcher stays reachable with no window.
//! Left click opens the window; the menu lists running games with a stop item each.

use std::sync::Mutex;

use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::state::AppState;
use crate::{launch, window};

const TRAY_ID: &str = "main";
const OPEN: &str = "open";
const EXIT: &str = "exit";
const STOP_PREFIX: &str = "stop:";

/// Makes the menu follow the latest snapshot even when several refreshes overlap.
static REFRESH: Mutex<()> = Mutex::new(());

fn build_menu(app: &AppHandle, running: &[(String, String)]) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, OPEN, "Открыть _k0t1k", true, None::<&str>)?;
    let stops = running
        .iter()
        .map(|(id, name)| MenuItem::with_id(app, format!("{STOP_PREFIX}{id}"), format!("Остановить: {name}"), true, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let stop_items: Vec<&dyn IsMenuItem<tauri::Wry>> = stops.iter().map(|item| item as _).collect();
    let games = Submenu::with_items(app, "Запущенные игры", !stops.is_empty(), &stop_items)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, EXIT, "Выход", true, None::<&str>)?;
    Menu::with_items(app, &[&open, &games, &separator, &exit])
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let icon = app.default_window_icon().cloned().ok_or_else(|| tauri::Error::AssetNotFound("window icon".to_owned()))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("_k0t1k")
        .menu(&build_menu(app, &[])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN => window::open(app),
            EXIT => window::request_exit(app),
            id => {
                if let Some(instance_id) = id.strip_prefix(STOP_PREFIX)
                    && let Err(error) = launch::kill(app, &app.state::<AppState>(), instance_id)
                {
                    tracing::warn!("cannot stop {instance_id}: {error}");
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::open(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Rebuilds the menu from the running games. Called on every state change, and only then. It runs
/// on a blocking thread, after the caller released the lock on the running games.
pub fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _refreshing = REFRESH.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            return;
        };
        let mut running: Vec<(String, String)> =
            app.state::<AppState>().running().iter().map(|(id, entry)| (id.clone(), entry.name.clone())).collect();
        running.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        if let Err(error) = build_menu(&app, &running).and_then(|menu| tray.set_menu(Some(menu))) {
            tracing::error!("cannot update the tray menu: {error}");
        }
    });
}
