use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use k0t1k_core::content::{Clients, curseforge};
use k0t1k_core::instance::InstanceManager;
use k0t1k_core::launch::GameHandle;
use k0t1k_core::paths::Paths;
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

use crate::datadir::DataLocation;
use crate::error::{AppError, CommandResult};
use crate::events::{LogLine, RunState};

/// Baked in by build.rs from the environment or `.env`; absent in builds without a key.
const CURSEFORGE_API_KEY: Option<&str> = option_env!("K0T1K_CURSEFORGE_API_KEY");

/// Lines of one instance's log kept for a window that is opened later; matches the UI's own cap.
pub const MAX_LOG_LINES: usize = 5000;

/// A game that is being prepared, running or being stopped.
pub struct RunEntry {
    pub name: String,
    pub state: RunState,
    pub handle: Option<GameHandle>,
    /// Becomes `true` when a kill is requested; preparation waits on it to abort promptly.
    pub cancel: watch::Sender<bool>,
}

impl RunEntry {
    pub fn kill_requested(&self) -> bool {
        *self.cancel.borrow()
    }
}

/// A background install of an instance's game files.
pub struct GameInstall {
    pub title: String,
    pub task: JoinHandle<()>,
}

/// Cheap to clone: every field is shared, so background tasks take their own copy.
#[derive(Clone)]
pub struct AppState {
    pub paths: Paths,
    pub location: DataLocation,
    pub http: reqwest::Client,
    pub instances: Arc<InstanceManager>,
    pub clients: Arc<Clients>,
    running: Arc<Mutex<HashMap<String, RunEntry>>>,
    /// Background game installs by instance id, so deleting the instance can stop its download.
    game_installs: Arc<Mutex<HashMap<String, GameInstall>>>,
    /// Recent log lines per instance. A session's log stays after the game exits, until the next launch.
    game_logs: Arc<Mutex<HashMap<String, VecDeque<LogLine>>>>,
    /// Numbers log lines across all instances, so the UI can tell which lines it already has.
    log_seq: Arc<AtomicU64>,
    /// Set while the main window is gone or hidden only because a game started (see `window`).
    pub window_released: Arc<AtomicBool>,
    /// Held while game files and Java are installed: concurrent installs would write the same
    /// files, and a launch must wait for a background install of its own game instead of racing it.
    pub install_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(location: DataLocation) -> CommandResult<Self> {
        let paths = Paths::new(location.root.clone());
        let http = k0t1k_core::download::http_client();
        let key = CURSEFORGE_API_KEY.filter(|key| !key.is_empty());
        Ok(Self {
            instances: Arc::new(InstanceManager::new(paths.clone())),
            clients: Arc::new(Clients::new(http.clone(), key)?),
            paths,
            location,
            http,
            running: Arc::default(),
            game_installs: Arc::default(),
            game_logs: Arc::default(),
            log_seq: Arc::default(),
            window_released: Arc::default(),
            install_lock: Arc::default(),
        })
    }

    pub fn curseforge(&self) -> CommandResult<&curseforge::Client> {
        self.clients
            .curseforge
            .as_ref()
            .ok_or_else(|| AppError::new("CurseForge unavailable: this build has no CurseForge API key"))
    }

    pub fn running(&self) -> MutexGuard<'_, HashMap<String, RunEntry>> {
        self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn game_installs(&self) -> MutexGuard<'_, HashMap<String, GameInstall>> {
        self.game_installs.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_running(&self, instance_id: &str) -> bool {
        self.running().contains_key(instance_id)
    }

    pub fn kill_requested(&self, instance_id: &str) -> bool {
        self.running().get(instance_id).is_some_and(RunEntry::kill_requested)
    }

    /// Marks the instance as being prepared; fails when it is already running.
    /// The receiver turns `true` when a kill is requested.
    pub fn reserve(&self, instance_id: &str, name: &str) -> CommandResult<watch::Receiver<bool>> {
        let mut running = self.running();
        if running.contains_key(instance_id) {
            return Err(AppError::new("the instance is already running"));
        }
        let (cancel, killed) = watch::channel(false);
        running.insert(instance_id.to_owned(), RunEntry { name: name.to_owned(), state: RunState::Preparing, handle: None, cancel });
        Ok(killed)
    }
}

impl AppState {
    /// Forgets the log of the previous session of the instance.
    pub fn reset_log(&self, instance_id: &str) {
        self.logs().remove(instance_id);
    }

    /// Numbers the line and keeps it in the instance's ring buffer.
    pub fn record_log(&self, instance_id: &str, stream: &'static str, line: &str) -> LogLine {
        let entry = LogLine { seq: self.log_seq.fetch_add(1, Ordering::Relaxed), stream, line: line.to_owned() };
        let mut logs = self.logs();
        let buffer = logs.entry(instance_id.to_owned()).or_default();
        if buffer.len() == MAX_LOG_LINES {
            buffer.pop_front();
        }
        buffer.push_back(entry.clone());
        entry
    }

    pub fn log_lines(&self, instance_id: &str) -> Vec<LogLine> {
        self.logs().get(instance_id).map(|buffer| buffer.iter().cloned().collect()).unwrap_or_default()
    }

    fn logs(&self) -> MutexGuard<'_, HashMap<String, VecDeque<LogLine>>> {
        self.game_logs.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datadir::DataDirSource;

    #[test]
    fn log_buffer_keeps_the_newest_lines_and_resets_per_launch() {
        let root = std::env::temp_dir().join("k0t1k-state-test");
        let location = DataLocation { root: root.clone(), exe_dir: root, source: DataDirSource::Custom };
        let state = AppState::new(location).unwrap();

        for n in 0..MAX_LOG_LINES + 3 {
            state.record_log("a", "stdout", &n.to_string());
        }
        state.record_log("b", "stderr", "other");
        let lines = state.log_lines("a");
        assert_eq!(lines.len(), MAX_LOG_LINES);
        assert_eq!(lines[0].line, "3");
        assert!(lines.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert_eq!(state.log_lines("b").len(), 1);

        state.reset_log("a");
        assert!(state.log_lines("a").is_empty());
        assert_eq!(state.log_lines("b").len(), 1);
    }
}
