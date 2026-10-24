//! Lifecycle of the main window, which is what makes the launcher cheap while a game runs: the web
//! view's processes cost far more memory than the Rust process, so the window is destroyed, not hidden.
//!
//! Rules:
//! - Closing the window (title bar button, Alt+F4) follows `behavior.on_close`: `tray` destroys the
//!   window and keeps the launcher in the tray, `exit` quits (asking first when a game is running).
//! - When a game starts (any session of any instance), `behavior.on_game_launch` destroys (`free_memory`) or hides (`hide_to_tray`)
//!   the window, if there is one, and marks it as released for the game.
//! - When the last session of all instances ends, a released window is created or shown again. Opening the window yourself
//!   (tray click, second launch) clears the mark, so a window you closed afterwards stays closed.
//! - The app exits only on an explicit request (`exit`); closing the last window never exits it.

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use k0t1k_core::settings::{GlobalSettings, OnClose, OnGameLaunch};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

use crate::datadir::DataDirSource;
use crate::state::{AppState, SessionId};
use crate::{launch, tray};

pub const MAIN: &str = "main";

const YES: &str = "Да";
const NO: &str = "Нет";
const CANCEL: &str = "Отмена";

/// How long exiting waits for killed games to end and their playtime to be saved.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// Serialises creating, showing and destroying the window, so two quick requests cannot both create it.
static WINDOW_OPS: Mutex<()> = Mutex::new(());

fn lock_ops() -> std::sync::MutexGuard<'static, ()> {
    WINDOW_OPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where the window was when it was last destroyed, so a re-created window opens in the same place
/// instead of the screen centre. Kept in memory only: a new launcher run starts centred as before.
#[derive(Clone, Copy)]
struct Placement {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    maximized: bool,
}

static LAST_PLACEMENT: Mutex<Option<Placement>> = Mutex::new(None);

fn last_placement() -> std::sync::MutexGuard<'static, Option<Placement>> {
    LAST_PLACEMENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Destroys the window after remembering its placement.
fn destroy_window(window: &WebviewWindow) -> tauri::Result<()> {
    let maximized = window.is_maximized()?;
    // a minimized window reports a position far off screen, which must not be restored
    if !window.is_minimized()? {
        *last_placement() = Some(Placement { position: window.outer_position()?, size: window.inner_size()?, maximized });
    }
    window.destroy()
}

/// Window operations wait for the main thread, so they run on a blocking thread: callers are often
/// event handlers on the main thread itself.
fn background(app: &AppHandle, work: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || work(&app));
}

/// Created in code rather than in tauri.conf.json to keep WebView2's profile in the data directory
/// instead of `%LOCALAPPDATA%`.
pub fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let state = app.state::<AppState>();
    state.window_released.store(false, Ordering::SeqCst);
    let placement = *last_placement();
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::default())
        .title("_k0t1k")
        .inner_size(1200.0, 760.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        .shadow(true)
        // a window put back where it was stays hidden until it is there, so it does not flash in the centre
        .visible(placement.is_none())
        .data_directory(state.location.webview_dir())
        .build()?;
    if let Some(placement) = placement {
        window.set_size(placement.size)?;
        window.set_position(placement.position)?;
        if placement.maximized {
            window.maximize()?;
        }
        window.show()?;
        window.set_focus()?;
    }
    Ok(window)
}

fn show_or_create(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        return create(app).map(|_| ());
    };
    app.state::<AppState>().window_released.store(false, Ordering::SeqCst);
    window.show()?;
    window.unminimize()?;
    window.set_focus()
}

/// Shows the window, creating it when it was destroyed.
pub fn open(app: &AppHandle) {
    background(app, |app| {
        let _ops = lock_ops();
        if let Err(error) = show_or_create(app) {
            tracing::error!("cannot open the launcher window: {error}");
        }
    });
}

/// Exits the launcher. The window is destroyed first and the WebView2 profile lock awaited: exiting with a
/// live webview leaves its msedgewebview2 processes running as orphans that keep the profile (and memory).
/// Blocks for up to a few seconds, so call it from a blocking thread.
pub fn exit(app: &AppHandle) {
    destroy(app);
    let lockfile = app.state::<AppState>().location.webview_dir().join("EBWebView").join("lockfile");
    for _ in 0..50 {
        match std::fs::remove_file(&lockfile) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => std::thread::sleep(Duration::from_millis(100)),
            _ => break,
        }
    }
    app.exit(0);
}

fn destroy(app: &AppHandle) {
    let _ops = lock_ops();
    if let Some(window) = app.get_webview_window(MAIN)
        && let Err(error) = destroy_window(&window)
    {
        tracing::error!("cannot close the launcher window: {error}");
    }
}

pub fn on_game_started(app: &AppHandle, mode: OnGameLaunch) {
    if mode == OnGameLaunch::KeepOpen {
        return;
    }
    background(app, move |app| {
        let _ops = lock_ops();
        let state = app.state::<AppState>();
        // The game may already be over: its stop handler could have run before this one.
        if state.running().is_empty() {
            return;
        }
        let Some(window) = app.get_webview_window(MAIN) else {
            return;
        };
        state.window_released.store(true, Ordering::SeqCst);
        let result = if mode == OnGameLaunch::FreeMemory { destroy_window(&window) } else { window.hide() };
        if let Err(error) = result {
            tracing::error!("cannot release the launcher window: {error}");
        }
    });
}

pub fn on_game_stopped(app: &AppHandle) {
    background(app, |app| {
        let _ops = lock_ops();
        let state = app.state::<AppState>();
        if state.running().is_empty()
            && state.window_released.load(Ordering::SeqCst)
            && let Err(error) = show_or_create(app)
        {
            tracing::error!("cannot restore the launcher window: {error}");
        }
    });
}

/// The window's close button or an OS close request.
pub fn request_close(app: &AppHandle) {
    background(app, |app| {
        // The onboarding screen has no tray icon to come back from.
        if app.state::<AppState>().location.source == DataDirSource::Onboarding {
            exit(app);
            return;
        }
        let on_close = match GlobalSettings::load(&app.state::<AppState>().paths) {
            Ok(settings) => settings.behavior.on_close,
            Err(error) => {
                tracing::error!("cannot read the settings, closing to the tray: {error}");
                OnClose::Tray
            }
        };
        match on_close {
            OnClose::Tray => destroy(app),
            OnClose::Exit => ask_and_exit(app),
        }
    });
}

/// The tray's "Exit".
pub fn request_exit(app: &AppHandle) {
    background(app, ask_and_exit);
}

/// Exits, after asking what to do with running games. Runs on a blocking thread: the dialog blocks.
fn ask_and_exit(app: &AppHandle) {
    let names: Vec<String> = tray::session_labels(&app.state::<AppState>()).into_iter().map(|(_, label)| label).collect();
    if names.is_empty() {
        exit(app);
        return;
    }
    let answer = app
        .dialog()
        .message(format!("Запущено: {}.\nЗакрыть игру тоже?", names.join(", ")))
        .title("_k0t1k")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(YES.to_owned(), NO.to_owned(), CANCEL.to_owned()))
        .blocking_show_with_result();
    match answer {
        MessageDialogResult::Custom(label) if label == YES => {
            kill_games(app);
            exit(app);
        }
        MessageDialogResult::Custom(label) if label == NO => exit(app),
        _ => {}
    }
}

/// Kills every session and waits until its session has ended, so the playtime is saved before exiting.
fn kill_games(app: &AppHandle) {
    let state = app.state::<AppState>();
    let ids: Vec<SessionId> = state.running().keys().copied().collect();
    for id in ids {
        if let Err(error) = launch::kill(app, &state, id) {
            tracing::warn!("cannot stop session {id}: {error}");
        }
    }
    let ended = tauri::async_runtime::block_on(tokio::time::timeout(KILL_WAIT, async {
        while !state.running().is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }));
    if ended.is_err() {
        tracing::warn!("games did not stop within {KILL_WAIT:?}, exiting anyway");
    }
}
