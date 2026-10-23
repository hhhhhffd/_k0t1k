use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use k0t1k_core::content::{Clients, curseforge};
use k0t1k_core::instance::InstanceManager;
use k0t1k_core::launch::GameHandle;
use k0t1k_core::paths::Paths;

use crate::error::{AppError, CommandResult};
use crate::events::RunState;

/// Baked in by build.rs from the environment or `.env`; absent in builds without a key.
const CURSEFORGE_API_KEY: Option<&str> = option_env!("K0T1K_CURSEFORGE_API_KEY");

/// A game that is being prepared, running or being stopped.
pub struct RunEntry {
    pub state: RunState,
    pub handle: Option<GameHandle>,
    /// Kill was requested while the game was still being prepared.
    pub kill_requested: bool,
}

/// Cheap to clone: every field is shared, so background tasks take their own copy.
#[derive(Clone)]
pub struct AppState {
    pub paths: Paths,
    pub http: reqwest::Client,
    pub instances: Arc<InstanceManager>,
    pub clients: Arc<Clients>,
    running: Arc<Mutex<HashMap<String, RunEntry>>>,
}

impl AppState {
    pub fn new() -> CommandResult<Self> {
        let root = Paths::default_root().ok_or_else(|| AppError::new("cannot determine the data directory"))?;
        let paths = Paths::new(root);
        let http = k0t1k_core::download::http_client();
        let key = CURSEFORGE_API_KEY.filter(|key| !key.is_empty());
        Ok(Self {
            instances: Arc::new(InstanceManager::new(paths.clone())),
            clients: Arc::new(Clients::new(http.clone(), key)?),
            paths,
            http,
            running: Arc::default(),
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

    pub fn is_running(&self, instance_id: &str) -> bool {
        self.running().contains_key(instance_id)
    }

    /// Marks the instance as being prepared; fails when it is already running.
    pub fn reserve(&self, instance_id: &str) -> CommandResult<()> {
        let mut running = self.running();
        if running.contains_key(instance_id) {
            return Err(AppError::new("the instance is already running"));
        }
        running.insert(instance_id.to_owned(), RunEntry { state: RunState::Preparing, handle: None, kill_requested: false });
        Ok(())
    }
}
