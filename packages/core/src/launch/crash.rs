//! Finding the crash report a game left behind.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The newest crash report written at or after `since`: a Minecraft report in `crash-reports/` or a
/// JVM crash log (`hs_err_pid*.log`) in the game directory.
pub fn find_crash_report(game_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    let reports = candidates(&game_dir.join("crash-reports"), |name| name.starts_with("crash-") && name.ends_with(".txt"));
    let jvm = candidates(game_dir, |name| name.starts_with("hs_err_pid") && name.ends_with(".log"));
    reports.into_iter().chain(jvm).filter(|(_, modified)| *modified >= since).max_by_key(|(_, modified)| *modified).map(|(path, _)| path)
}

fn candidates(dir: &Path, wanted: impl Fn(&str) -> bool) -> Vec<(PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_str().is_some_and(&wanted))
        .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.modified().ok()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn write_at(path: &Path, age: Duration) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn newest_report_after_the_start_wins() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        let since = SystemTime::now() - Duration::from_secs(100);
        write_at(&game.join("crash-reports/crash-old.txt"), Duration::from_secs(500));
        write_at(&game.join("crash-reports/crash-a.txt"), Duration::from_secs(50));
        write_at(&game.join("crash-reports/crash-b.txt"), Duration::from_secs(10));
        write_at(&game.join("crash-reports/notes.txt"), Duration::from_secs(1));
        assert_eq!(find_crash_report(game, since), Some(game.join("crash-reports/crash-b.txt")));

        write_at(&game.join("hs_err_pid42.log"), Duration::from_secs(5));
        assert_eq!(find_crash_report(game, since), Some(game.join("hs_err_pid42.log")));
    }

    #[test]
    fn nothing_found_without_a_fresh_report() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_crash_report(dir.path(), SystemTime::now()), None);
        write_at(&dir.path().join("crash-reports/crash-old.txt"), Duration::from_secs(500));
        assert_eq!(find_crash_report(dir.path(), SystemTime::now() - Duration::from_secs(10)), None);
    }
}
