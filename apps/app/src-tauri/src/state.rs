use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use k0t1k_core::auth::DeviceCode;
use k0t1k_core::content::{Clients, curseforge};
use k0t1k_core::instance::InstanceManager;
use k0t1k_core::launch::GameHandle;
use k0t1k_core::mclogs::mask_secrets;
use k0t1k_core::paths::Paths;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

use crate::datadir::DataLocation;
use crate::error::{AppError, CommandResult};
use crate::events::{LogLine, RunState};

/// Baked in by build.rs from the environment or `.env`; absent in builds without a key.
const CURSEFORGE_API_KEY: Option<&str> = option_env!("K0T1K_CURSEFORGE_API_KEY");

/// Azure application id for Microsoft sign-in, baked in by build.rs; absent in builds without one.
pub const MSA_CLIENT_ID: Option<&str> = option_env!("K0T1K_MSA_CLIENT_ID");

/// Lines of one session's log kept for a window that is opened later; matches the UI's own cap.
pub const MAX_LOG_LINES: usize = 5000;

/// Finished sessions of one instance whose logs are kept (until the app exits) for the Logs tab.
pub const KEPT_FINISHED_SESSIONS: usize = 3;

/// Identifies one launch of an instance; numbers go up for the whole app run, so ids are never reused.
pub type SessionId = u64;

/// A game that is being prepared, running or being stopped. One entry per launch: an instance may
/// have any number of them at once.
pub struct RunEntry {
    pub instance_id: String,
    pub name: String,
    /// 1 for the first launch of the instance in this app run, 2 for the next, ...
    pub number: u32,
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

/// How a session ended when the game crashed.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashInfo {
    /// `None` when a signal ended the process.
    pub exit_code: Option<i32>,
    /// A Minecraft crash report or JVM crash log written during the session.
    pub crash_report: Option<String>,
}

/// Log and identity of a session, kept after the game exits (see [`KEPT_FINISHED_SESSIONS`]).
struct SessionLog {
    instance_id: String,
    number: u32,
    started_at: DateTime<Utc>,
    finished: bool,
    lines: VecDeque<LogLine>,
    crash: Option<CrashInfo>,
    /// The UI showed or dismissed the crash; a window opened later does not show it again.
    crash_dismissed: bool,
    /// Values that must not leave the machine in an uploaded log.
    secrets: Vec<String>,
}

#[derive(Default)]
struct SessionLogs {
    sessions: HashMap<SessionId, SessionLog>,
    /// Launches so far per instance in this app run.
    launches: HashMap<String, u32>,
}

/// What the UI needs to know about a session; `state` is `stopped` for a finished one whose log is kept.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub instance_id: String,
    pub number: u32,
    pub state: RunState,
    pub started_at: DateTime<Utc>,
    /// Set for a crashed session until the UI dismisses it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crash: Option<CrashInfo>,
}

/// A Microsoft sign-in waiting for the user to enter the device code.
pub struct MicrosoftLogin {
    pub device: DeviceCode,
    /// Becomes `true` when the sign-in is cancelled.
    pub cancel: watch::Sender<bool>,
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
    running: Arc<Mutex<HashMap<SessionId, RunEntry>>>,
    next_session: Arc<AtomicU64>,
    /// Background game installs by instance id, so deleting the instance can stop its download.
    game_installs: Arc<Mutex<HashMap<String, GameInstall>>>,
    game_logs: Arc<Mutex<SessionLogs>>,
    /// Numbers log lines across all sessions, so the UI can tell which lines it already has.
    log_seq: Arc<AtomicU64>,
    /// Set while the main window is gone or hidden only because a game started (see `window`).
    pub window_released: Arc<AtomicBool>,
    /// Held while game files and Java are installed: concurrent installs would write the same
    /// files, and a launch must wait for a background install of its own game instead of racing it.
    pub install_lock: Arc<tokio::sync::Mutex<()>>,
    /// The pending Microsoft sign-in, if any (one at a time).
    pub microsoft_login: Arc<Mutex<Option<MicrosoftLogin>>>,
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
            next_session: Arc::new(AtomicU64::new(1)),
            game_installs: Arc::default(),
            game_logs: Arc::default(),
            log_seq: Arc::default(),
            window_released: Arc::default(),
            install_lock: Arc::default(),
            microsoft_login: Arc::default(),
        })
    }

    pub fn curseforge(&self) -> CommandResult<&curseforge::Client> {
        self.clients
            .curseforge
            .as_ref()
            .ok_or_else(|| AppError::new("CurseForge unavailable: this build has no CurseForge API key"))
    }

    pub fn running(&self) -> MutexGuard<'_, HashMap<SessionId, RunEntry>> {
        self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn game_installs(&self) -> MutexGuard<'_, HashMap<String, GameInstall>> {
        self.game_installs.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_running(&self, instance_id: &str) -> bool {
        self.running().values().any(|entry| entry.instance_id == instance_id)
    }

    pub fn kill_requested(&self, session_id: SessionId) -> bool {
        self.running().get(&session_id).is_some_and(RunEntry::kill_requested)
    }

    /// Starts a new session of the instance in the `Preparing` state; other sessions of the same
    /// instance may be running. The receiver turns `true` when a kill is requested.
    pub fn reserve(&self, instance_id: &str, name: &str) -> (SessionId, watch::Receiver<bool>) {
        let session_id = self.next_session.fetch_add(1, Ordering::Relaxed);
        let started_at = Utc::now();
        let mut logs = self.logs();
        let launches = logs.launches.entry(instance_id.to_owned()).or_default();
        *launches += 1;
        let number = *launches;
        logs.sessions.insert(
            session_id,
            SessionLog {
                instance_id: instance_id.to_owned(),
                number,
                started_at,
                finished: false,
                lines: VecDeque::new(),
                crash: None,
                crash_dismissed: false,
                secrets: Vec::new(),
            },
        );
        drop(logs);
        let (cancel, killed) = watch::channel(false);
        self.running().insert(
            session_id,
            RunEntry {
                instance_id: instance_id.to_owned(),
                name: name.to_owned(),
                number,
                state: RunState::Preparing,
                handle: None,
                cancel,
            },
        );
        (session_id, killed)
    }

    /// Running sessions followed by finished ones whose log is kept.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let running = self.running();
        let logs = self.logs();
        let mut sessions: Vec<SessionInfo> = logs
            .sessions
            .iter()
            .map(|(&session_id, log)| SessionInfo {
                session_id,
                instance_id: log.instance_id.clone(),
                number: log.number,
                state: running.get(&session_id).map_or(RunState::Stopped, |entry| entry.state),
                started_at: log.started_at,
                crash: log.crash.clone().filter(|_| !log.crash_dismissed),
            })
            .collect();
        sessions.sort_by_key(|session| session.session_id);
        sessions
    }

    /// Number of the session within its instance; `None` once the session is gone.
    pub fn session_number(&self, session_id: SessionId) -> Option<u32> {
        self.logs().sessions.get(&session_id).map(|log| log.number)
    }

    /// Marks the session's log as finished and drops the oldest finished logs of its instance.
    pub fn finish_log(&self, session_id: SessionId) {
        let mut logs = self.logs();
        let Some(log) = logs.sessions.get_mut(&session_id) else {
            return;
        };
        log.finished = true;
        let instance_id = log.instance_id.clone();
        let mut finished: Vec<SessionId> =
            logs.sessions.iter().filter(|(_, log)| log.finished && log.instance_id == instance_id).map(|(&id, _)| id).collect();
        finished.sort_unstable();
        let excess = finished.len().saturating_sub(KEPT_FINISHED_SESSIONS);
        for id in &finished[..excess] {
            logs.sessions.remove(id);
        }
    }

    /// Numbers the line, masks the session's secrets in it and keeps it in the session's ring buffer.
    pub fn record_log(&self, session_id: SessionId, stream: &'static str, line: &str) -> LogLine {
        let mut logs = self.logs();
        let session = logs.sessions.get_mut(&session_id);
        let line = match &session {
            Some(log) => mask_secrets(line, &log.secrets),
            None => line.to_owned(),
        };
        let entry = LogLine { seq: self.log_seq.fetch_add(1, Ordering::Relaxed), stream, line };
        if let Some(log) = session {
            if log.lines.len() == MAX_LOG_LINES {
                log.lines.pop_front();
            }
            log.lines.push_back(entry.clone());
        }
        entry
    }

    pub fn set_crash(&self, session_id: SessionId, crash: CrashInfo) {
        if let Some(log) = self.logs().sessions.get_mut(&session_id) {
            log.crash = Some(crash);
        }
    }

    pub fn dismiss_crash(&self, session_id: SessionId) {
        if let Some(log) = self.logs().sessions.get_mut(&session_id) {
            log.crash_dismissed = true;
        }
    }

    pub fn add_log_secret(&self, session_id: SessionId, secret: &str) {
        if let Some(log) = self.logs().sessions.get_mut(&session_id) {
            log.secrets.push(secret.to_owned());
        }
    }

    /// Log lines, the crash report path and the secrets to mask, for an upload; `None` once the session is gone.
    pub fn upload_material(&self, session_id: SessionId) -> Option<(Vec<String>, Option<String>, Vec<String>)> {
        let logs = self.logs();
        let log = logs.sessions.get(&session_id)?;
        let lines = log.lines.iter().map(|entry| entry.line.clone()).collect();
        Some((lines, log.crash.as_ref().and_then(|crash| crash.crash_report.clone()), log.secrets.clone()))
    }

    pub fn log_lines(&self, session_id: SessionId) -> Vec<LogLine> {
        self.logs().sessions.get(&session_id).map(|log| log.lines.iter().cloned().collect()).unwrap_or_default()
    }

    fn logs(&self) -> MutexGuard<'_, SessionLogs> {
        self.game_logs.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datadir::DataDirSource;

    #[test]
    fn logs_are_kept_per_session_and_only_the_newest_finished_ones() {
        let root = std::env::temp_dir().join("k0t1k-state-test");
        let location = DataLocation { root: root.clone(), exe_dir: root, source: DataDirSource::Custom };
        let state = AppState::new(location).unwrap();

        let (a, _) = state.reserve("inst", "Inst");
        let (b, _) = state.reserve("inst", "Inst");
        let (other, _) = state.reserve("other", "Other");
        assert_eq!((state.session_number(a), state.session_number(b), state.session_number(other)), (Some(1), Some(2), Some(1)));
        assert!(state.is_running("inst"));

        for n in 0..MAX_LOG_LINES + 3 {
            state.record_log(a, "stdout", &n.to_string());
        }
        state.record_log(b, "stderr", "other");
        let lines = state.log_lines(a);
        assert_eq!(lines.len(), MAX_LOG_LINES);
        assert_eq!(lines[0].line, "3");
        assert!(lines.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert_eq!(state.log_lines(b).len(), 1);

        // Finished sessions keep their logs, but only the newest few per instance.
        let mut ended = vec![a, b];
        for _ in 0..KEPT_FINISHED_SESSIONS {
            ended.push(state.reserve("inst", "Inst").0);
        }
        for id in &ended {
            state.running().remove(id);
            state.finish_log(*id);
        }
        assert!(state.log_lines(a).is_empty());
        assert!(state.session_number(b).is_none());
        assert!(state.session_number(ended[2]).is_some());
        assert!(state.is_running("other"));
        assert!(!state.is_running("inst"));
        let infos = state.sessions();
        assert_eq!(infos.iter().filter(|info| info.state == RunState::Stopped).count(), KEPT_FINISHED_SESSIONS);
        assert!(infos.iter().any(|info| info.session_id == other && info.state == RunState::Preparing));
    }

    #[test]
    fn registered_secrets_are_masked_in_recorded_lines() {
        let root = std::env::temp_dir().join("k0t1k-state-secret-test");
        let location = DataLocation { root: root.clone(), exe_dir: root, source: DataDirSource::Custom };
        let state = AppState::new(location).unwrap();
        let (session, _) = state.reserve("inst", "Inst");
        state.add_log_secret(session, "secret-access-token");
        state.add_log_secret(session, "0");
        let entry = state.record_log(session, "stdout", "(Session ID is token:secret-access-token:uuid) 0");
        assert_eq!(entry.line, "(Session ID is token:***:uuid) 0");
        assert_eq!(state.log_lines(session)[0].line, entry.line);
    }
}
