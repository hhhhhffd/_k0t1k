//! Launcher log: `logs/launcher.log` for this run, `launcher.1.log` ... for the previous ones.

use std::fs::File;
use std::path::Path;
use std::sync::Mutex;

use k0t1k_core::paths::Paths;
use tracing_subscriber::filter::LevelFilter;

const LOG_NAME: &str = "launcher";
/// Previous runs kept besides the current one.
const OLD_RUNS: u32 = 5;
/// Overrides the level (`error`, `warn`, `info`, `debug`, `trace`).
const LEVEL_VARIABLE: &str = "K0T1K_LOG";

fn log_file(dir: &Path, number: u32) -> std::path::PathBuf {
    match number {
        0 => dir.join(format!("{LOG_NAME}.log")),
        n => dir.join(format!("{LOG_NAME}.{n}.log")),
    }
}

/// Shifts every log one place up (the oldest is dropped) and opens a fresh `launcher.log`.
fn rotate_and_open(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    for number in (0..=OLD_RUNS).rev() {
        let from = log_file(dir, number);
        if !from.exists() {
            continue;
        }
        if number == OLD_RUNS {
            std::fs::remove_file(&from)?;
        } else {
            std::fs::rename(&from, log_file(dir, number + 1))?;
        }
    }
    File::create(log_file(dir, 0))
}

/// Starts logging into the data directory. Failing to log must not stop the launcher, so problems
/// only go to stderr.
pub fn init(paths: &Paths) {
    let file = match rotate_and_open(&paths.logs_dir()) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("cannot open the launcher log: {error}");
            return;
        }
    };
    let level = std::env::var(LEVEL_VARIABLE).ok().and_then(|value| value.parse().ok()).unwrap_or(LevelFilter::INFO);
    let subscriber = tracing_subscriber::fmt().with_writer(Mutex::new(file)).with_ansi(false).with_max_level(level).finish();
    if let Err(error) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("cannot start the launcher log: {error}");
        return;
    }
    // Release builds abort on a panic, so this is the only trace of it.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));
    tracing::info!("_k0t1k {} started, data directory {}", env!("CARGO_PKG_VERSION"), paths.root().display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_runs_are_kept_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        for run in 0..OLD_RUNS + 3 {
            let mut file = rotate_and_open(dir.path()).unwrap();
            std::io::Write::write_all(&mut file, run.to_string().as_bytes()).unwrap();
        }
        let read = |number| std::fs::read_to_string(log_file(dir.path(), number)).unwrap();
        assert_eq!(read(0), "7");
        assert_eq!(read(1), "6");
        assert_eq!(read(OLD_RUNS), "2");
        assert!(!log_file(dir.path(), OLD_RUNS + 1).exists());
    }
}
