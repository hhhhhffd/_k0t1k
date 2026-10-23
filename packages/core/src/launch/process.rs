use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::error::{Error, IoContext, Result};

use super::LaunchCommand;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long output readers may outlive the game process. A grandchild that inherited the pipes
/// (e.g. a wrapper's daemon) would otherwise keep them open and the exit would never be reported.
const READER_GRACE: Duration = Duration::from_secs(2);

/// Windows has no process groups: a job object lets the whole process tree be killed at once,
/// so a wrapper command cannot leave the actual game running.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

// A job handle is just a kernel object reference, valid on any thread.
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Job {
    /// `None` (with a warning) when the job cannot be set up; killing then reaches only the child.
    fn containing(child: &tokio::process::Child) -> Option<Job> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};

        let process = child.raw_handle()?;
        // SAFETY: plain Win32 calls; the job handle is closed on every failure path.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                tracing::warn!("cannot create a job object: {}", std::io::Error::last_os_error());
                return None;
            }
            if AssignProcessToJobObject(job, process as _) == 0 {
                tracing::warn!("cannot assign the game to a job object: {}", std::io::Error::last_os_error());
                CloseHandle(job);
                return None;
            }
            Some(Job(job))
        }
    }

    fn terminate(&self) {
        // SAFETY: the handle is a valid job object owned by `self`.
        if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) } == 0 {
            tracing::warn!("cannot terminate the game's job object: {}", std::io::Error::last_os_error());
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by `self` and closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

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

/// Waits for an output reader after the process exited; gives up on it after `grace`.
async fn finish_reader(mut reader: JoinHandle<std::io::Result<()>>, grace: Duration) {
    match tokio::time::timeout(grace, &mut reader).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => tracing::error!("reading game output failed: {error}"),
        Ok(Err(error)) => tracing::error!("game output task failed: {error}"),
        Err(_) => {
            tracing::warn!("game output is still open {grace:?} after the process exited; giving up on it");
            reader.abort();
        }
    }
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

    #[cfg(windows)]
    let job = Job::containing(&child);

    let (kill, mut kill_requests) = mpsc::unbounded_channel();
    let (exit_sender, exit) = watch::channel(None);
    tokio::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            Some(()) = kill_requests.recv() => {
                #[cfg(windows)]
                if let Some(job) = &job {
                    job.terminate();
                }
                match child.kill().await {
                    Ok(()) => child.wait().await,
                    Err(error) => Err(error),
                }
            }
        };
        for reader in readers {
            finish_reader(reader, READER_GRACE).await;
        }
        exit_sender.send_replace(Some(status.map(|status| status.code()).map_err(|error| error.to_string())));
    });

    Ok(GameHandle { pid, kill, exit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reader_with_open_pipe_is_given_up_after_the_grace_period() {
        // The write half stays alive, like a grandchild holding the game's pipe.
        let (_writer, reader) = tokio::io::duplex(64);
        let handle = pump_lines(reader, LogStream::Stdout, Arc::new(|_| {}));
        tokio::time::timeout(Duration::from_secs(5), finish_reader(handle, Duration::from_millis(50)))
            .await
            .expect("finish_reader must not wait for an open pipe");
    }
}
