//! Launch sequence: prepare the game files, run the game, track its state and playtime.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use std::time::SystemTime;

use chrono::Utc;
use k0t1k_core::auth;
use k0t1k_core::install::{check_installed, extract_natives, install_game, reconstruct_assets};
use k0t1k_core::instance::servers::{pull_shared, push_shared};
use k0t1k_core::instance::{EffectiveSettings, Instance, SharedFolder};
use k0t1k_core::java::ensure_java;
use k0t1k_core::launch::crash::find_crash_report;
use k0t1k_core::launch::tuning::tune;
use k0t1k_core::launch::{LaunchCommand, LaunchOptions, LogFn, LogStream, WindowSize, build_command, spawn_game};
use k0t1k_core::profile::LaunchProfile;
use k0t1k_core::progress::ProgressFn;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::mclogs::{self, UploadedLog};
use k0t1k_core::settings::{Account, GlobalSettings, OnGameLaunch, total_memory_mib};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::process::Command;
use tokio::sync::watch;

use crate::error::{AppError, CommandResult, blocking};
use crate::events::{self, GameLog, LogLine, RunState, progress_fn, tracked};
use crate::state::{AppState, CrashInfo, MSA_CLIENT_ID, SessionId, SessionInfo};
use crate::{cmdline, discord, game, window};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    account_id: Option<String>,
    server: Option<String>,
) -> CommandResult<SessionId> {
    let state = state.inner().clone();
    let manager = state.instances.clone();
    let id = instance_id.clone();
    let (instance, global) = blocking(move || Ok((manager.get(&id)?, GlobalSettings::load(manager.paths())?))).await?;

    let account = match &account_id {
        Some(id) => global.accounts.iter().find(|account| &account.id == id),
        None => global.selected_account(),
    }
    .ok_or_else(|| AppError::new("no account selected: add an account in the settings"))?
    .clone();
    // An explicit server wins over the one stored with the instance.
    let join_server = server.or_else(|| instance.join_server.clone());
    let settings = instance.effective_settings(&global);

    // Every launch is its own session: the instance may already be running, and concurrent
    // preparations are fine (installs are serialised by `install_lock`).
    let (session_id, kill_requests) = state.reserve(&instance_id, &instance.name);
    let number = state.session_number(session_id).unwrap_or(1);
    events::instance_state(&app, session_id, &instance_id, number, RunState::Preparing, None);
    tauri::async_runtime::spawn(run_session(
        app,
        state,
        session_id,
        instance,
        settings,
        account,
        join_server,
        global.behavior.on_game_launch,
        global.discord.enabled,
        kill_requests,
    ));
    Ok(session_id)
}

#[tauri::command]
pub fn kill_session(app: AppHandle, state: State<'_, AppState>, session_id: SessionId) -> CommandResult<()> {
    kill(&app, &state, session_id)
}

/// Stops every session of the instance.
#[tauri::command]
pub fn kill_instance(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    let sessions: Vec<SessionId> =
        state.running().iter().filter(|(_, entry)| entry.instance_id == instance_id).map(|(&id, _)| id).collect();
    if sessions.is_empty() {
        return Err(AppError::new("the instance is not running"));
    }
    for session_id in sessions {
        // A session may have ended since the list was made; that is the goal anyway.
        if let Err(error) = kill(&app, &state, session_id) {
            tracing::debug!("{error}");
        }
    }
    Ok(())
}

/// Stops the game of the session, or cancels its preparation.
pub fn kill(app: &AppHandle, state: &AppState, session_id: SessionId) -> CommandResult<()> {
    let mut running = state.running();
    let entry = running.get_mut(&session_id).ok_or_else(|| AppError::new("the session is not running"))?;
    entry.cancel.send_replace(true);
    entry.state = RunState::Stopping;
    if let Some(handle) = &entry.handle {
        handle.kill();
    }
    events::instance_state(app, session_id, &entry.instance_id, entry.number, RunState::Stopping, None);
    Ok(())
}

/// Running sessions and finished ones whose log is still kept (state `stopped`).
#[tauri::command]
pub fn list_run_states(state: State<'_, AppState>) -> Vec<SessionInfo> {
    state.sessions()
}

/// Recent log lines of a session, for a window opened after the game started.
#[tauri::command]
pub fn get_game_logs(state: State<'_, AppState>, session_id: SessionId) -> Vec<LogLine> {
    state.log_lines(session_id)
}

/// The crash dialog of the session was shown or closed; a window opened later does not show it again.
#[tauri::command]
pub fn dismiss_crash(state: State<'_, AppState>, session_id: SessionId) {
    state.dismiss_crash(session_id);
}

/// Opens the crash report of a crashed session with the system's default program.
#[tauri::command]
pub fn open_crash_report(app: AppHandle, state: State<'_, AppState>, session_id: SessionId) -> CommandResult<()> {
    let (_, report, _) = state.upload_material(session_id).ok_or_else(|| AppError::new("the session is gone"))?;
    let report = report.ok_or_else(|| AppError::new("the session left no crash report"))?;
    app.opener().open_path(report, None::<&str>)?;
    Ok(())
}

/// Uploads the session's log (and its crash report, if any) to mclo.gs. Only runs on the user's request.
#[tauri::command]
pub async fn upload_game_log(state: State<'_, AppState>, session_id: SessionId) -> CommandResult<UploadedLog> {
    let (lines, report, secrets) = state.upload_material(session_id).ok_or_else(|| AppError::new("the session is gone"))?;
    let report = match report {
        Some(path) => {
            let bytes = tokio::fs::read(&path).await.map_err(|error| AppError::new(format!("cannot read {path}: {error}")))?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        None => None,
    };
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
    let content = mclogs::prepare_content(&lines, report.as_deref(), &secrets);
    Ok(mclogs::upload(&state.http, &content).await?)
}

/// Clears the run entry and tells the UI the session stopped when the session task ends without
/// [`SessionGuard::end`], i.e. when it panics; otherwise the session would stay "running" forever.
struct SessionGuard {
    app: AppHandle,
    state: AppState,
    session_id: SessionId,
    instance_id: String,
    number: u32,
    /// Natives and the JVM argfile of this session.
    dir: PathBuf,
    log: GameLog,
    ended: bool,
}

impl SessionGuard {
    /// Forgets the running game and tells the UI it stopped; playtime must be saved before this.
    fn end(&mut self, message: &str, exit_code: Option<i32>) {
        self.ended = true;
        self.log.line("launcher", message);
        // Windows may refuse to delete a loaded DLL while the process is still shutting down; the
        // next launch of the instance removes what is left (see `remove_stale_sessions`).
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!("cannot remove the session files {}: {error}", self.dir.display()),
        }
        self.state.running().remove(&self.session_id);
        self.state.finish_log(self.session_id);
        discord::session_stopped(self.session_id);
        events::instance_state(&self.app, self.session_id, &self.instance_id, self.number, RunState::Stopped, exit_code);
        window::on_game_stopped(&self.app);
    }
}

/// Removes the files of sessions that no longer run (left by a crash or by a failed removal), keeping
/// those of live sessions, which includes the ones still preparing.
async fn remove_stale_sessions(state: &AppState, instance: &Instance, log: &GameLog) {
    let sessions_dir = instance.sessions_dir(&state.paths);
    let mut entries = match tokio::fs::read_dir(&sessions_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => return log.line("launcher", &format!("Не удалось просмотреть {}: {error}", sessions_dir.display())),
    };
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => return,
            Err(error) => return log.line("launcher", &format!("Не удалось просмотреть {}: {error}", sessions_dir.display())),
        };
        let live = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<SessionId>().ok())
            .is_some_and(|id| state.running().contains_key(&id));
        if live {
            continue;
        }
        let path = entry.path();
        let removed = match entry.file_type().await {
            Ok(kind) if kind.is_dir() => tokio::fs::remove_dir_all(&path).await,
            _ => tokio::fs::remove_file(&path).await,
        };
        if let Err(error) = removed {
            tracing::warn!("cannot remove stale session files {}: {error}", path.display());
        }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if !self.ended {
            self.end("Сессия запуска завершилась аварийно", None);
        }
    }
}

async fn wait_killed(kill_requests: &mut watch::Receiver<bool>) {
    // The sender lives in the run entry, which is removed only when the session ends.
    let _ = kill_requests.wait_for(|requested| *requested).await;
}

#[allow(clippy::too_many_arguments)] // one argument per piece of the launch, none of them worth a wrapper type
async fn run_session(
    app: AppHandle,
    state: AppState,
    session_id: SessionId,
    instance: Instance,
    settings: EffectiveSettings,
    account: Account,
    join_server: Option<String>,
    on_game_launch: OnGameLaunch,
    show_in_discord: bool,
    mut kill_requests: watch::Receiver<bool>,
) {
    let id = instance.id.clone();
    let task = format!("launch:{session_id}");
    let title = format!("Запуск: {}", instance.name);
    let post_exit = settings.commands.post_exit.clone();
    let progress = progress_fn(&app, &task, &title);
    let log = GameLog::new(&app, &state, session_id);
    let dir = instance.sessions_dir(&state.paths).join(session_id.to_string());
    let mut session = SessionGuard {
        app: app.clone(),
        state: state.clone(),
        session_id,
        instance_id: id.clone(),
        number: state.session_number(session_id).unwrap_or(1),
        dir: dir.clone(),
        log: log.clone(),
        ended: false,
    };

    // Dropping `prepare` on a kill also stops its downloads and kills a running pre-launch hook.
    let preparation = async {
        tokio::select! {
            prepared = prepare(&state, &log, &instance, &settings, account, join_server, &dir, &task, &progress) => prepared,
            () = wait_killed(&mut kill_requests) => Err(AppError::new("запуск отменён")),
        }
    };
    let command = match tracked(&app, &task, &title, preparation).await {
        Ok(command) => command,
        Err(_) if state.kill_requested(session_id) => return session.end("Запуск отменён", None),
        Err(error) => return session.end(&format!("Запуск не удался: {error}"), None),
    };
    if state.kill_requested(session_id) {
        return session.end("Запуск отменён", None);
    }

    // A shared server list is copied into the game dir for this run and merged back after it.
    let shared_servers = if instance.shared_folders.contains(&SharedFolder::Servers) {
        let (paths, game_dir) = (state.paths.clone(), instance.game_dir(&state.paths));
        match blocking(move || pull_shared(&paths, &game_dir)).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                log.line("launcher", &format!("Не удалось подготовить общий список серверов: {error}"));
                None
            }
        }
    } else {
        None
    };

    let on_log: LogFn = {
        let log = log.clone();
        Arc::new(move |line| {
            let stream = match line.stream {
                LogStream::Stdout => "stdout",
                LogStream::Stderr => "stderr",
            };
            log.line(stream, &line.line);
        })
    };
    let handle = match spawn_game(&command, on_log) {
        Ok(handle) => handle,
        Err(error) => return session.end(&format!("Не удалось запустить игру: {}", AppError::from(error)), None),
    };
    let started_at = Utc::now();
    let spawned_at = SystemTime::now();
    let killed_at_spawn = {
        let mut running = state.running();
        match running.get_mut(&session_id) {
            Some(entry) => {
                entry.handle = Some(handle.clone());
                if entry.kill_requested() {
                    handle.kill();
                } else {
                    entry.state = RunState::Running;
                    // Emitted under the lock so a concurrent kill's `Stopping` cannot be overwritten.
                    events::instance_state(&app, session_id, &id, entry.number, RunState::Running, None);
                }
                entry.kill_requested()
            }
            None => false,
        }
    };

    if !killed_at_spawn {
        window::on_game_started(&app, on_game_launch);
        if show_in_discord {
            discord::session_started(session_id, &instance, started_at);
        }
    }

    let exit_code = match handle.wait().await {
        Ok(code) => code,
        Err(error) => {
            log.line("launcher", &format!("Не удалось дождаться завершения игры: {error}"));
            None
        }
    };
    let ended_at = Utc::now();

    if let Some(snapshot) = shared_servers {
        let (paths, game_dir) = (state.paths.clone(), instance.game_dir(&state.paths));
        if let Err(error) = blocking(move || push_shared(&paths, &game_dir, snapshot.as_deref())).await {
            log.line("launcher", &format!("Не удалось сохранить общий список серверов: {error}"));
        }
    }

    // A game killed right at its start never really ran.
    if !killed_at_spawn {
        let manager = state.instances.clone();
        let recorded = {
            let id = id.clone();
            blocking(move || manager.record_session(&id, started_at, ended_at).map(|_| ())).await
        };
        if let Err(error) = recorded {
            log.line("launcher", &format!("Не удалось сохранить время игры: {error}"));
        }
    }
    // Windows reports a killed process as exit code 1, so the code alone cannot tell a kill apart.
    let killed = state.kill_requested(session_id);
    let crash = if killed || exit_code == Some(0) {
        None
    } else {
        let game_dir = instance.game_dir(&state.paths);
        let report = blocking(move || Ok(find_crash_report(&game_dir, spawned_at))).await.ok().flatten();
        if let Some(report) = &report {
            log.line("launcher", &format!("Найден отчёт о сбое: {}", report.display()));
        }
        Some(CrashInfo { exit_code, crash_report: report.map(|path| path.display().to_string()) })
    };
    let message = if killed {
        "Игра была завершена принудительно".to_owned()
    } else {
        match exit_code {
            Some(code) => format!("Игра завершилась с кодом {code}"),
            None => "Игра была завершена сигналом".to_owned(),
        }
    };
    if let Some(crash) = &crash {
        state.set_crash(session_id, crash.clone());
    }
    session.end(&message, exit_code);
    if let Some(crash) = &crash {
        events::game_crashed(&app, session_id, &id, crash);
    }

    if !post_exit.is_empty() {
        let game_dir = instance.game_dir(&state.paths);
        if let Err(error) = run_hook("post-exit", &post_exit, &game_dir).await {
            log.line("launcher", &error.to_string());
        }
    }
}

#[allow(clippy::too_many_arguments)] // see `run_session`
async fn prepare(
    state: &AppState,
    log: &GameLog,
    instance: &Instance,
    settings: &EffectiveSettings,
    account: Account,
    join_server: Option<String>,
    session_dir: &Path,
    task: &str,
    progress: &ProgressFn,
) -> CommandResult<LaunchCommand> {
    let paths = &state.paths;

    let (components, online) = game::resolve(paths, instance.components.clone()).await?;
    // Only the Java requirements are needed from this profile; natives depend on the chosen Java.
    let preliminary = LaunchProfile::build(&components, RuntimeContext::host(game::host_java_arch()))?;
    let installing = state.install_lock.lock().await;
    let java = ensure_java(
        paths,
        &state.http,
        &preliminary.compatible_java_majors,
        preliminary.compatible_java_name.as_deref(),
        &settings.java,
        progress,
    )
    .await?;
    if !preliminary.is_java_compatible(java.major) {
        let warning = format!(
            "Предупреждение: Java {} может не подходить этой версии игры (нужна одна из {:?})",
            java.major, preliminary.compatible_java_majors
        );
        log.line("launcher", &warning);
    }

    let profile = LaunchProfile::build(&components, RuntimeContext::host(&java.arch))?;
    let assets = if online {
        install_game(&state.http, paths, &profile, task, progress).await?
    } else {
        log.line("launcher", "Нет сети: используются уже загруженные файлы игры");
        check_installed(paths, &profile).await?
    };
    drop(installing);

    let game_dir = instance.game_dir(paths);
    remove_stale_sessions(state, instance, log).await;
    // The argfile is written next to this directory, so it belongs to the session as well.
    let natives_dir = session_dir.join("natives");
    extract_natives(paths, &profile, &natives_dir).await?;
    reconstruct_assets(paths, &assets, &game_dir).await?;

    if !settings.commands.pre_launch.is_empty() {
        run_hook("pre-launch", &settings.commands.pre_launch, &game_dir).await?;
    }

    let extra_jvm_args = cmdline::split(&settings.jvm_args)?;
    let (min_memory_mb, max_memory_mb) = if settings.memory.auto {
        let tuning = tune(total_memory_mib());
        log.line("launcher", &format!("Автоподбор памяти (как в Prism): {}-{} МиБ", tuning.min_mib, tuning.max_mib));
        (tuning.min_mib, tuning.max_mib)
    } else {
        (settings.memory.min_mib, settings.memory.max_mib)
    };
    // Resolved last, so a long install or hook cannot leave the token expired. Refreshes expiring tokens;
    // a failure (for example an expired sign-in) fails the launch with its message.
    let identity = auth::resolve_launch_identity(paths, &state.http, MSA_CLIENT_ID, &account).await?;
    // Masked in every log line from now on (before the command line is logged and the game starts) and in
    // an uploaded log; masking ignores tokens too short to be real (offline placeholders).
    log.add_secret(&identity.account.access_token);
    log.line("launcher", &format!("Аккаунт: {} ({})", account.name, identity.account.user_type));

    let window = (!settings.window.maximized).then_some(WindowSize { width: settings.window.width, height: settings.window.height });
    let options = LaunchOptions {
        java_path: java.path,
        java_major: java.major,
        game_dir,
        natives_dir,
        profile_name: instance.name.clone(),
        min_memory_mb,
        max_memory_mb,
        java_agent_args: identity.java_agent_args,
        extra_jvm_args,
        window,
        account: identity.account,
        env: settings.env.clone(),
        wrapper: cmdline::split(&settings.commands.wrapper)?,
        join_server,
    };
    let command = build_command(paths, &profile, &assets, &options).await?;
    log.line("launcher", &format!("Команда запуска: {}", command.display_redacted()));
    Ok(command)
}

/// Runs a pre-launch or post-exit command line in the game directory and waits for it.
async fn run_hook(name: &str, command_line: &str, game_dir: &Path) -> CommandResult<()> {
    let args = cmdline::split(command_line)?;
    let Some((program, args)) = args.split_first() else {
        return Ok(());
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(game_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let status = command
        .status()
        .await
        .map_err(|error| AppError::new(format!("cannot run the {name} command {program}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::new(format!("the {name} command failed with {status}")))
    }
}
