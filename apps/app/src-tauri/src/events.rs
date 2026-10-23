//! Events sent to the UI: `progress`, `game-log` (batched) and `instance-state`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use k0t1k_core::progress::{ProgressEvent, ProgressFn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::error::CommandResult;
use crate::state::{AppState, SessionId};
use crate::{tray, window};

/// Core reports every downloaded file; the UI does not need more than a few updates per second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Serialize)]
struct ProgressPayload {
    #[serde(flatten)]
    event: ProgressEvent,
    title: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    finished: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Game output can reach tens of thousands of lines in a couple of minutes, so lines are sent in batches.
const LOG_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Serialize)]
pub struct LogLine {
    pub seq: u64,
    pub stream: &'static str,
    pub line: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameLogPayload {
    session_id: SessionId,
    lines: Vec<LogLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Preparing,
    Running,
    Stopping,
    Stopped,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStatePayload {
    pub session_id: SessionId,
    pub instance_id: String,
    pub number: u32,
    pub state: RunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

fn emit<P: Serialize + Clone>(app: &AppHandle, event: &str, payload: P) {
    if let Err(error) = app.emit(event, payload) {
        tracing::warn!("cannot emit {event}: {error}");
    }
}

fn payload(task: &str, title: &str, stage: &str, done: u64, total: u64) -> ProgressPayload {
    ProgressPayload {
        event: ProgressEvent { task: task.to_owned(), stage: stage.to_owned(), done, total },
        title: title.to_owned(),
        finished: false,
        error: None,
    }
}

/// Progress callback for core calls. Every event is reported under `task`, whatever task id core
/// used internally, so one operation shows up as one row in the UI.
pub fn progress_fn(app: &AppHandle, task: &str, title: &str) -> ProgressFn {
    let app = app.clone();
    let task = task.to_owned();
    let title = title.to_owned();
    let last_emit = Mutex::new(Instant::now());
    Arc::new(move |event: ProgressEvent| {
        let mut last = last_emit.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if event.done != 0 && event.done != event.total && last.elapsed() < PROGRESS_INTERVAL {
            return;
        }
        *last = Instant::now();
        emit(&app, "progress", ProgressPayload { event: ProgressEvent { task: task.clone(), ..event }, title: title.clone(), finished: false, error: None });
    })
}

pub fn progress_step(app: &AppHandle, task: &str, title: &str, stage: &str, done: u64, total: u64) {
    emit(app, "progress", payload(task, title, stage, done, total));
}

/// Ends the task row of an operation that was stopped on purpose.
pub fn cancelled(app: &AppHandle, task: &str, title: &str) {
    let mut end = payload(task, title, "Отменено", 1, 1);
    end.finished = true;
    emit(app, "progress", end);
}

/// Runs `work` and ends the task row with `finished`, or with `error` when it fails.
pub async fn tracked<T>(app: &AppHandle, task: &str, title: &str, work: impl Future<Output = CommandResult<T>>) -> CommandResult<T> {
    let result = work.await;
    let mut end = payload(task, title, "Готово", 1, 1);
    match &result {
        Ok(_) => end.finished = true,
        Err(error) => {
            end.event.stage = "Ошибка".to_owned();
            end.error = Some(error.to_string());
        }
    }
    emit(app, "progress", end);
    result
}

/// Log of one launch session. Lines sent through it are kept in the session's ring buffer (see
/// `AppState::record_log`) and reach the UI as `game-log` events carrying the session id and arrays of lines, at most one
/// event per [`LOG_INTERVAL`]. Clones share the same queue; the sender task ends, after sending what
/// is left, when every clone is dropped. With nothing queued it sleeps on the channel, not on a timer.
#[derive(Clone)]
pub struct GameLog {
    state: AppState,
    session_id: SessionId,
    lines: mpsc::UnboundedSender<LogLine>,
}

impl GameLog {
    pub fn new(app: &AppHandle, state: &AppState, session_id: SessionId) -> Self {
        let (lines, mut queue) = mpsc::unbounded_channel();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(first) = queue.recv().await {
                let mut batch = vec![first];
                tokio::time::sleep(LOG_INTERVAL).await;
                while let Ok(line) = queue.try_recv() {
                    batch.push(line);
                }
                // The lines are in the ring buffer; a window that opens later fetches them from there.
                if app.get_webview_window(window::MAIN).is_some() {
                    emit(&app, "game-log", GameLogPayload { session_id, lines: batch });
                }
            }
        });
        Self { state: state.clone(), session_id, lines }
    }

    /// `stream` is `stdout`, `stderr` or `launcher` (the launcher's own messages).
    pub fn line(&self, stream: &'static str, line: &str) {
        let entry = self.state.record_log(self.session_id, stream, line);
        if self.lines.send(entry).is_err() {
            tracing::warn!("game log is closed, dropping: {line}");
        }
    }
}

/// `number` is the session's number within its instance (see `RunEntry::number`).
pub fn instance_state(app: &AppHandle, session_id: SessionId, instance_id: &str, number: u32, state: RunState, exit_code: Option<i32>) {
    emit(app, "instance-state", InstanceStatePayload { session_id, instance_id: instance_id.to_owned(), number, state, exit_code });
    tray::refresh(app);
}
