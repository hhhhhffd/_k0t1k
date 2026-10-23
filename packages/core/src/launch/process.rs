use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::error::{Error, IoContext, Result};

use super::LaunchCommand;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub stream: LogStream,
    pub line: String,
}

pub type LogFn = Arc<dyn Fn(LogLine) + Send + Sync>;

/// `Some` once the process has exited: its exit code (`None` when killed by a signal) or the
/// reason waiting for it failed.
type ExitState = Option<std::result::Result<Option<i32>, String>>;

/// A running game. Clones share the same process, so one can wait while another kills.
#[derive(Clone)]
pub struct GameHandle {
    pid: Option<u32>,
    kill: mpsc::UnboundedSender<()>,
    exit: watch::Receiver<ExitState>,
}

impl GameHandle {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Asks the supervisor to kill the process; [`GameHandle::wait`] returns once it is gone.
    pub fn kill(&self) {
        // A closed channel means the supervisor has finished, i.e. the process already exited.
        let _ = self.kill.send(());
    }

    /// Waits for the process to exit and for all its output to be delivered.
    /// Returns the exit code, `None` when the process was terminated by a signal.
    pub async fn wait(&self) -> Result<Option<i32>> {
        let mut exit = self.exit.clone();
        let state = exit
            .wait_for(Option::is_some)
            .await
            .map_err(|_| Error::Other("the game process supervisor stopped unexpectedly".to_owned()))?;
        match &*state {
            Some(Ok(code)) => Ok(*code),
            Some(Err(message)) => Err(Error::Other(format!("waiting for the game process failed: {message}"))),
            None => Err(Error::Other("the game process has not exited".to_owned())),
        }
    }
}

fn pump_lines<R>(stream: R, kind: LogStream, on_log: LogFn) -> JoinHandle<std::io::Result<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            if reader.read_until(b'\n', &mut buffer).await? == 0 {
                return Ok(());
            }
            while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                buffer.pop();
            }
            on_log(LogLine { stream: kind, line: String::from_utf8_lossy(&buffer).into_owned() });
        }
    })
}

/// Starts the game with cwd = game dir and a clean environment. Must be called inside a tokio
/// runtime. Output lines are delivered to `on_log` from separate tasks, one per stream.
pub fn spawn_game(command: &LaunchCommand, on_log: LogFn) -> Result<GameHandle> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for name in super::DROPPED_ENV {
        process.env_remove(name);
    }
    process.envs(&command.env);
    #[cfg(windows)]
    process.creation_flags(CREATE_NO_WINDOW);

    let mut child = process.spawn().at(&command.program)?;
    let pid = child.id();
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Error::Other("the game process has no output pipes".to_owned()));
    };
    let readers =
        [pump_lines(stdout, LogStream::Stdout, on_log.clone()), pump_lines(stderr, LogStream::Stderr, on_log)];

    let (kill, mut kill_requests) = mpsc::unbounded_channel();
    let (exit_sender, exit) = watch::channel(None);
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            Some(()) = kill_requests.recv() => match child.kill().await {
                Ok(()) => child.wait().await,
                Err(error) => Err(error),
            },
        };
        for reader in readers {
            match reader.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!("reading game output failed: {error}"),
                Err(error) => tracing::error!("game output task failed: {error}"),
            }
        }
        exit_sender.send_replace(Some(status.map(|status| status.code()).map_err(|error| error.to_string())));
    });

    Ok(GameHandle { pid, kill, exit })
}
