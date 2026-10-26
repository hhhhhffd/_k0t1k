//! First-run onboarding, relocation and uninstall (Windows).
//!
//! A downloaded executable usually sits in Desktop or Downloads, where portable data would spill
//! over the user's files. When the executable's folder is such a place and the user has not decided
//! yet, the launcher starts on a temporary data root (`datadir::resolve`) and the UI shows only the
//! onboarding screen. The user then
//! - installs: the exe is copied to the target folder and started there with
//!   `--wait-for <pid> --finish-relocation <old exe>`; the old process exits, which releases the
//!   single-instance mutex, and the new one deletes the old exe, moves leftovers of ours, creates
//!   shortcuts and starts normally (problems are shown by the UI, see [`RelocationReport`]);
//! - or keeps it here: the choice is stored in `settings.json` next to the exe (or only
//!   `--portable-here` for this run) and the launcher restarts on the real data root.
//!
//! If anything fails before the new process is started, the old exe still exists and works.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use k0t1k_core::paths::Paths;
use k0t1k_core::placement::{self, Clutter, KnownFolders, RemovalPlan};
use k0t1k_core::settings::GlobalSettings;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::datadir::{self, DataDirSource};
use crate::error::{AppError, CommandResult, blocking};
use crate::platform;
use crate::state::AppState;

/// Name of the executable in an install folder.
const INSTALLED_EXE: &str = if cfg!(windows) { "_k0t1k.exe" } else { "_k0t1k" };
const SHORTCUT_NAME: &str = "_k0t1k.lnk";
const ONBOARDING_ROOT_PREFIX: &str = "_k0t1k-onboarding-";

/// How long a restarted launcher waits for its predecessor, and for the old exe to become deletable.
const WAIT_FOR_EXIT: Duration = Duration::from_secs(30);
const DELETE_ATTEMPTS: u32 = 10;
const DELETE_PAUSE: Duration = Duration::from_millis(300);

pub(crate) const WAIT_FOR: &str = "--wait-for";
const FINISH_RELOCATION: &str = "--finish-relocation";
pub(crate) const PORTABLE_HERE: &str = "--portable-here";
const DESKTOP_SHORTCUT: &str = "--desktop-shortcut";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StartupArgs {
    /// Process to wait for before starting, so its single-instance mutex is gone.
    pub wait_for: Option<u32>,
    /// Do not ask where to live in this run.
    pub portable_here: bool,
    /// The executable this one replaces.
    pub finish_relocation: Option<PathBuf>,
    pub desktop_shortcut: bool,
}

impl StartupArgs {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some(WAIT_FOR) => {
                    let pid = args.next().and_then(|value| value.to_str()?.parse().ok());
                    parsed.wait_for = Some(pid.ok_or_else(|| format!("{WAIT_FOR} needs a process id"))?);
                }
                Some(FINISH_RELOCATION) => {
                    let old_exe = args.next().ok_or_else(|| format!("{FINISH_RELOCATION} needs a path"))?;
                    parsed.finish_relocation = Some(PathBuf::from(old_exe));
                }
                Some(PORTABLE_HERE) => parsed.portable_here = true,
                Some(DESKTOP_SHORTCUT) => parsed.desktop_shortcut = true,
                _ => {}
            }
        }
        Ok(parsed)
    }
}

/// What the previous launcher process left to do, shown once by the UI.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocationReport {
    /// The folder the launcher now lives in, when it was just installed.
    pub installed_to: Option<String>,
    /// Entries of the old folder that were moved here.
    pub moved: Vec<String>,
    /// Things that could not be moved, deleted or created, each with the path where it is.
    pub problems: Vec<String>,
    #[serde(skip)]
    pub created_shortcuts: Vec<PathBuf>,
}

/// The report waiting for the UI; taken by `get_relocation_report`.
#[derive(Default)]
pub struct PendingReport(Mutex<Option<RelocationReport>>);

impl PendingReport {
    pub fn new(report: Option<RelocationReport>) -> Self {
        Self(Mutex::new(report))
    }

    pub fn report(&self) -> std::sync::MutexGuard<'_, Option<RelocationReport>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Runs before the window and the single-instance plugin exist: waits for the previous process and,
/// after an install, cleans up behind it. Returns what the UI should tell the user, if anything.
pub fn run_startup_tasks(args: &StartupArgs) -> Option<RelocationReport> {
    let mut report = RelocationReport::default();
    if let Some(pid) = args.wait_for {
        match platform::wait_for_exit(pid, WAIT_FOR_EXIT) {
            Ok(true) => {}
            Ok(false) => report.problems.push(format!("the previous launcher process {pid} did not exit within {WAIT_FOR_EXIT:?}")),
            Err(error) => report.problems.push(error.to_string()),
        }
    }
    if let Some(old_exe) = &args.finish_relocation {
        finish_relocation(old_exe, args, &mut report);
    }
    (report.installed_to.is_some() || !report.problems.is_empty()).then_some(report)
}

fn finish_relocation(old_exe: &Path, args: &StartupArgs, report: &mut RelocationReport) {
    let new_exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            report.problems.push(format!("cannot determine the new executable: {error}"));
            return;
        }
    };
    let Some(new_dir) = new_exe.parent() else {
        report.problems.push(format!("{} has no folder", new_exe.display()));
        return;
    };
    report.installed_to = Some(new_dir.display().to_string());
    if old_exe == new_exe {
        report.problems.push("the old and the new executable are the same file, nothing was cleaned up".to_owned());
        return;
    }
    if let Some(old_dir) = old_exe.parent()
        && !placement::same_dir(old_dir, new_dir)
    {
        move_owned_entries(old_dir, new_dir, report);
    }
    delete_with_retries(old_exe, report);
    let (created, problems) = create_shortcuts_for(&new_exe, args.desktop_shortcut);
    report.created_shortcuts = created;
    report.problems.extend(problems);
}

/// Data that was next to the old exe (normally none, as the launcher ran on a temporary root).
fn move_owned_entries(old_dir: &Path, new_dir: &Path, report: &mut RelocationReport) {
    for source in placement::owned_entries(old_dir) {
        let Some(name) = source.file_name() else { continue };
        let destination = new_dir.join(name);
        if destination.symlink_metadata().is_ok() {
            report.problems.push(format!("{} was not moved: {} already exists", source.display(), destination.display()));
            continue;
        }
        match move_path(&source, &destination) {
            Ok(()) => report.moved.push(name.to_string_lossy().into_owned()),
            Err(error) => report.problems.push(format!("{} was not moved to {}: {error}", source.display(), destination.display())),
        }
    }
}

/// Renames, and copies then deletes when the destination is on another drive.
fn move_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    match std::fs::rename(source, destination) {
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            if let Err(error) = copy_recursively(source, destination) {
                // Do not leave a half copy that would look like the real data.
                let cleanup = if destination.is_dir() { std::fs::remove_dir_all(destination) } else { std::fs::remove_file(destination) };
                return Err(match cleanup {
                    Err(cleanup_error) if cleanup_error.kind() != std::io::ErrorKind::NotFound => {
                        std::io::Error::other(format!("{error}; the partial copy {} could not be removed: {cleanup_error}", destination.display()))
                    }
                    _ => error,
                });
            }
            if source.is_dir() { std::fs::remove_dir_all(source) } else { std::fs::remove_file(source) }
        }
        other => other,
    }
}

/// Links (junctions, symlinks) are refused: copying them would silently turn them into other things.
fn copy_recursively(source: &Path, destination: &Path) -> std::io::Result<()> {
    let kind = source.symlink_metadata()?.file_type();
    if kind.is_symlink() {
        return Err(std::io::Error::other(format!("{} is a link and cannot be copied to another drive", source.display())));
    }
    if kind.is_dir() {
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(source, destination)?;
    }
    Ok(())
}

/// The old exe may be held for a moment by the process that just exited or by an antivirus scan.
fn delete_with_retries(file: &Path, report: &mut RelocationReport) {
    for attempt in 1..=DELETE_ATTEMPTS {
        match std::fs::remove_file(file) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if attempt == DELETE_ATTEMPTS => {
                report.problems.push(format!("the old launcher {} could not be deleted, delete it by hand: {error}", file.display()));
            }
            Err(_) => std::thread::sleep(DELETE_PAUSE),
        }
    }
}

fn shortcut_paths(desktop: bool) -> (Vec<PathBuf>, Vec<String>) {
    let mut paths = Vec::new();
    let mut problems = Vec::new();
    if desktop {
        match dirs::desktop_dir() {
            Some(dir) => paths.push(dir.join(SHORTCUT_NAME)),
            None => problems.push("cannot find the Desktop folder for a shortcut".to_owned()),
        }
    }
    (paths, problems)
}

/// Creates the requested shortcuts to `exe`; returns the created ones and the failures.
fn create_shortcuts_for(exe: &Path, desktop: bool) -> (Vec<PathBuf>, Vec<String>) {
    let (paths, mut problems) = shortcut_paths(desktop);
    let mut created = Vec::new();
    for lnk in paths {
        let result = match lnk.parent() {
            Some(dir) => std::fs::create_dir_all(dir).map_err(AppError::from).and_then(|()| platform::create_shortcut(&lnk, exe)),
            None => Err(AppError::new(format!("{} has no folder", lnk.display()))),
        };
        match result {
            Ok(()) => created.push(lnk),
            Err(error) => problems.push(error.to_string()),
        }
    }
    (created, problems)
}

/// Remembers shortcuts so an uninstall can remove them.
pub fn record_shortcuts(paths: &Paths, created: &[PathBuf]) -> k0t1k_core::Result<()> {
    if created.is_empty() {
        return Ok(());
    }
    GlobalSettings::update(paths, |settings| {
        for lnk in created {
            if !settings.onboarding.created_shortcuts.contains(lnk) {
                settings.onboarding.created_shortcuts.push(lnk.clone());
            }
        }
    })
    .map(|_| ())
}

// ---- startup decision ----

pub fn clutter(exe_dir: &Path) -> Option<Clutter> {
    placement::classify(exe_dir, &KnownFolders::detect())
}

/// Whether the user still has to decide where the launcher lives.
pub fn needed(exe_dir: &Path) -> CommandResult<bool> {
    match clutter(exe_dir) {
        None => Ok(false),
        // The temp folder is wiped sooner or later, so keeping the launcher there is no option.
        Some(Clutter::Temp) => Ok(true),
        Some(Clutter::UserFolder) => {
            let settings = GlobalSettings::load(&Paths::new(exe_dir))?;
            Ok(!settings.onboarding.keep_portable_at.as_deref().is_some_and(|kept| placement::same_dir(kept, exe_dir)))
        }
    }
}

/// The data root of an onboarding run: nothing is written next to the executable until the user decides.
pub fn temporary_root() -> PathBuf {
    std::env::temp_dir().join(format!("{ONBOARDING_ROOT_PREFIX}{}", std::process::id()))
}

/// Removes onboarding roots of earlier runs in the background. Right after a relocation the previous
/// process's WebView2 helpers still hold their files for a few seconds, so this retries for ~30 s.
/// Called once the single-instance check has passed, so no other onboarding run is alive.
pub fn sweep_stale_roots(current: &Path) {
    const ATTEMPTS: u32 = 20;
    let current = current.to_path_buf();
    std::thread::spawn(move || {
        for attempt in 1..=ATTEMPTS {
            if remove_stale_roots(&current, attempt == ATTEMPTS) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    });
}

/// One sweep pass; returns whether no stale root is left. Failures are only logged on the last pass.
fn remove_stale_roots(current: &Path, log_failures: bool) -> bool {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return true };
    let mut clean = true;
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with(ONBOARDING_ROOT_PREFIX)
            && path != current
            && let Err(error) = std::fs::remove_dir_all(&path)
        {
            clean = false;
            if log_failures {
                tracing::warn!("cannot remove the old onboarding folder {}: {error}", path.display());
            }
        }
    }
    clean
}

/// Best effort at exit; WebView2 may still hold files, in which case the next start sweeps them.
pub fn remove_own_temporary_root(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if state.location.source == DataDirSource::Onboarding
        && let Err(error) = std::fs::remove_dir_all(&state.location.root)
    {
        tracing::warn!("cannot remove {}: {error}", state.location.root.display());
    }
}

// ---- commands ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingInfo {
    /// `folder` (Desktop, Downloads or Documents) or `temp` (e.g. started inside a zip archive).
    reason: &'static str,
    exe_dir: String,
    /// Folder of the recommended install.
    default_target: String,
}

fn require_onboarding(state: &AppState) -> CommandResult<()> {
    if state.location.source == DataDirSource::Onboarding {
        Ok(())
    } else {
        Err(AppError::new("this is only available while the launcher asks where to live"))
    }
}

fn require_installed(state: &AppState) -> CommandResult<()> {
    if state.location.source == DataDirSource::Onboarding {
        return Err(AppError::new("choose where the launcher lives first"));
    }
    Ok(())
}

/// `None` when the launcher already knows where it lives.
#[tauri::command]
pub fn get_onboarding(state: State<'_, AppState>) -> CommandResult<Option<OnboardingInfo>> {
    if state.location.source != DataDirSource::Onboarding {
        return Ok(None);
    }
    let reason = match clutter(&state.location.exe_dir) {
        Some(Clutter::Temp) => "temp",
        _ => "folder",
    };
    let default_target = dirs::data_local_dir()
        .ok_or_else(|| AppError::new("cannot find the local application data folder"))?
        .join("Programs")
        .join(placement::INSTALL_SUBFOLDER);
    Ok(Some(OnboardingInfo { reason, exe_dir: state.location.exe_dir.display().to_string(), default_target: default_target.display().to_string() }))
}

/// Taken once: the notice about the move that just happened.
#[tauri::command]
pub fn get_relocation_report(pending: State<'_, PendingReport>) -> Option<RelocationReport> {
    pending.report().take()
}

/// Copies the running exe into the install folder for `chosen` (see `placement::install_target`).
fn install_exe(exe: &Path, chosen: &Path) -> CommandResult<PathBuf> {
    let target = placement::install_target(chosen, INSTALLED_EXE)?;
    let exe_dir = exe.parent().ok_or_else(|| AppError::new("cannot determine the executable's directory"))?;
    if placement::same_dir(&target, exe_dir) || placement::classify(&target, &KnownFolders::detect()).is_some() {
        return Err(AppError::new(format!("{} is not a good place for the launcher, pick another folder", target.display())));
    }
    datadir::ensure_writable(&target).map_err(|error| AppError::new(format!("{} is not writable: {error}", target.display())))?;
    let installed = target.join(INSTALLED_EXE);
    // Copy under another name first, so a failed copy never leaves a broken exe under the real one.
    let partial = target.join(format!("{INSTALLED_EXE}.part"));
    let result = std::fs::copy(exe, &partial).and_then(|_| std::fs::rename(&partial, &installed));
    if let Err(error) = result {
        let leftover = match std::fs::remove_file(&partial) {
            Err(cleanup_error) if cleanup_error.kind() != std::io::ErrorKind::NotFound => {
                format!("; the partial file {} could not be removed: {cleanup_error}", partial.display())
            }
            _ => String::new(),
        };
        return Err(AppError::new(format!("cannot copy the launcher to {}: {error}{leftover}", installed.display())));
    }
    Ok(installed)
}

/// Starts `exe` as a separate process that outlives this one.
pub(crate) fn spawn_detached(exe: &Path, args: &[OsString]) -> CommandResult<()> {
    let mut command = Command::new(exe);
    command.args(args);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command.spawn().map(drop).map_err(|error| AppError::new(format!("cannot start {}: {error}", exe.display())))
}

/// Installs into `target` (see `placement::install_target`) and restarts from there.
#[tauri::command]
pub async fn relocate(app: AppHandle, state: State<'_, AppState>, target: String, desktop_shortcut: bool) -> CommandResult<()> {
    require_onboarding(&state)?;
    let exe = std::env::current_exe()?;
    let source = exe.clone();
    let installed = tokio::task::spawn_blocking(move || install_exe(&source, Path::new(&target))).await??;

    let mut args = vec![OsString::from(WAIT_FOR), std::process::id().to_string().into(), FINISH_RELOCATION.into(), exe.into()];
    if desktop_shortcut {
        args.push(DESKTOP_SHORTCUT.into());
    }
    spawn_detached(&installed, &args)?;
    tokio::task::spawn_blocking(move || crate::window::exit(&app)).await?;
    Ok(())
}

/// Keeps the launcher where it is. With `remember` the choice is stored, otherwise it holds for this run only.
#[tauri::command]
pub async fn keep_portable_here(app: AppHandle, state: State<'_, AppState>, remember: bool) -> CommandResult<()> {
    require_onboarding(&state)?;
    let exe_dir = state.location.exe_dir.clone();
    if clutter(&exe_dir) == Some(Clutter::Temp) {
        return Err(AppError::new("the temp folder is cleaned up, install the launcher somewhere else"));
    }
    let mut args = vec![OsString::from(WAIT_FOR), std::process::id().to_string().into()];
    if remember {
        let paths = Paths::new(&exe_dir);
        blocking(move || {
            GlobalSettings::update(&paths, |settings| settings.onboarding.keep_portable_at = Some(exe_dir)).map(|_| ())
        })
        .await?;
    } else {
        args.push(PORTABLE_HERE.into());
    }
    spawn_detached(&std::env::current_exe()?, &args)?;
    tokio::task::spawn_blocking(move || crate::window::exit(&app)).await?;
    Ok(())
}

/// Creates a desktop shortcut to the running exe, for a launcher that was not installed by onboarding.
#[tauri::command]
pub async fn create_shortcuts(state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    require_installed(&state)?;
    let exe = std::env::current_exe()?;
    let paths = state.paths.clone();
    let (created, problems) = tokio::task::spawn_blocking(move || create_shortcuts_for(&exe, true)).await?;
    let names = created.iter().map(|lnk| lnk.display().to_string()).collect();
    blocking(move || record_shortcuts(&paths, &created)).await?;
    if problems.is_empty() { Ok(names) } else { Err(AppError::new(problems.join("; "))) }
}

#[derive(Serialize)]
pub struct UninstallPlan {
    /// Folders of the launcher; the launcher's own content in them is deleted, other files stay.
    folders: Vec<String>,
    shortcuts: Vec<String>,
}

fn removal_plan(state: &AppState) -> CommandResult<RemovalPlan> {
    require_installed(state)?;
    if !state.running().is_empty() {
        return Err(AppError::new("stop the running games before uninstalling"));
    }
    let settings = GlobalSettings::load(&state.paths)?;
    let exe = std::env::current_exe()?;
    Ok(RemovalPlan::new(state.paths.root(), &exe, &settings.onboarding.created_shortcuts, &KnownFolders::detect())?)
}

/// What an uninstall would delete; fails when it must not be done (a folder of the user's, running games).
#[tauri::command]
pub async fn get_uninstall_plan(state: State<'_, AppState>) -> CommandResult<UninstallPlan> {
    let state = state.inner().clone();
    let plan = tokio::task::spawn_blocking(move || removal_plan(&state)).await??;
    let display = |paths: &[PathBuf]| paths.iter().map(|path| path.display().to_string()).collect();
    Ok(UninstallPlan { folders: display(&plan.folders), shortcuts: display(&plan.shortcuts) })
}

/// A running exe cannot delete itself, so a detached `cmd` does it after this process has exited.
#[tauri::command]
pub async fn uninstall(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    let state = state.inner().clone();
    let script = tokio::task::spawn_blocking(move || removal_plan(&state).and_then(|plan| Ok(plan.cmd_script()?))).await??;
    spawn_cleanup(&script)?;
    tokio::task::spawn_blocking(move || crate::window::exit(&app)).await?;
    Ok(())
}

#[cfg(windows)]
fn spawn_cleanup(script: &str) -> CommandResult<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `raw_arg`: cmd has its own quoting, the quotes around the whole script are stripped by `/C`.
    // Run from the temp folder: a process's working directory cannot be deleted, and ours is the launcher folder.
    Command::new("cmd")
        .raw_arg(format!("/C \"{script}\""))
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
        .map_err(|error| AppError::new(format!("cannot start the cleanup: {error}")))
}

#[cfg(not(windows))]
fn spawn_cleanup(_script: &str) -> CommandResult<()> {
    Err(AppError::new("uninstalling is only implemented on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<StartupArgs, String> {
        StartupArgs::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn parses_relocation_arguments() {
        let args = parse(&["--wait-for", "42", "--finish-relocation", r"C:\Users\Ann\Desktop\k0t1k.exe", "--desktop-shortcut"]).unwrap();
        assert_eq!(args.wait_for, Some(42));
        assert_eq!(args.finish_relocation, Some(PathBuf::from(r"C:\Users\Ann\Desktop\k0t1k.exe")));
        assert!(args.desktop_shortcut && !args.portable_here);
        assert_eq!(parse(&["--portable-here", "unrelated"]).unwrap(), StartupArgs { portable_here: true, ..StartupArgs::default() });
        assert_eq!(parse(&[]).unwrap(), StartupArgs::default());
        assert!(parse(&["--wait-for", "soon"]).is_err());
        assert!(parse(&["--finish-relocation"]).is_err());
    }

    #[test]
    fn moves_files_and_folders_and_refuses_to_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("old"), root.path().join("new"));
        std::fs::create_dir_all(old.join("instances").join("a")).unwrap();
        std::fs::write(old.join("instances").join("a").join("options.txt"), b"x").unwrap();
        std::fs::write(old.join("settings.json"), b"{}").unwrap();
        std::fs::write(old.join("holiday.jpg"), b"not ours").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("settings.json"), b"{\"new\":1}").unwrap();

        let mut report = RelocationReport::default();
        move_owned_entries(&old, &new, &mut report);

        assert_eq!(report.moved, ["instances"]);
        assert_eq!(report.problems.len(), 1, "settings.json already exists in the new folder: {:?}", report.problems);
        assert!(new.join("instances").join("a").join("options.txt").exists());
        assert_eq!(std::fs::read(new.join("settings.json")).unwrap(), b"{\"new\":1}");
        assert!(old.join("settings.json").exists() && old.join("holiday.jpg").exists());
    }

    #[test]
    fn copy_fallback_copies_trees_and_stops_at_links() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::write(source.join("sub").join("f.txt"), b"x").unwrap();
        let copy = root.path().join("copy");
        copy_recursively(&source, &copy).unwrap();
        assert_eq!(std::fs::read(copy.join("sub").join("f.txt")).unwrap(), b"x");
    }
}
