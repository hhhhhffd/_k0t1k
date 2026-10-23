//! Events sent to the UI: `progress`, `game-log` and `instance-state`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use k0t1k_core::progress::{ProgressEvent, ProgressFn};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::CommandResult;

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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameLogPayload<'a> {
    instance_id: &'a str,
    stream: &'a str,
    line: &'a str,
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
    pub instance_id: String,
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

pub fn game_log(app: &AppHandle, instance_id: &str, stream: &str, line: &str) {
    emit(app, "game-log", GameLogPayload { instance_id, stream, line });
}

pub fn instance_state(app: &AppHandle, instance_id: &str, state: RunState, exit_code: Option<i32>) {
    emit(app, "instance-state", InstanceStatePayload { instance_id: instance_id.to_owned(), state, exit_code });
}
