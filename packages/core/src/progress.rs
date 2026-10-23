use std::sync::Arc;

use serde::Serialize;

/// Progress of a long-running task, forwarded to the UI as-is.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    /// Stable id of the task, e.g. `install:<instance id>`.
    pub task: String,
    /// Human-readable current step.
    pub stage: String,
    pub done: u64,
    pub total: u64,
}

pub type ProgressFn = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Progress sink that drops every event; for tests and callers without a UI.
pub fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}
