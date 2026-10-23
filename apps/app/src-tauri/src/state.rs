use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use k0t1k_core::content::{Clients, curseforge};
use k0t1k_core::instance::InstanceManager;
use k0t1k_core::launch::GameHandle;
use k0t1k_core::paths::Paths;
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

use crate::datadir::DataLocation;
use crate::error::{AppError, CommandResult};
use crate::events::RunState;

/// Baked in by build.rs from the environment or `.env`; absent in builds without a key.
const CURSEFORGE_API_KEY: Option<&str> = option_env!("K0T1K_CURSEFORGE_API_KEY");

/// A game that is being prepared, running or being stopped.
pub struct RunEntry {
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
    pub fn reserve(&self, instance_id: &str) -> CommandResult<watch::Receiver<bool>> {
        let mut running = self.running();
        if running.contains_key(instance_id) {
            return Err(AppError::new("the instance is already running"));
        }
        let (cancel, killed) = watch::channel(false);
        running.insert(instance_id.to_owned(), RunEntry { state: RunState::Preparing, handle: None, cancel });
        Ok(killed)
    }
}
