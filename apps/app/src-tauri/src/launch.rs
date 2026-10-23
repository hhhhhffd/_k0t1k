//! Launch sequence: prepare the game files, run the game, track its state and playtime.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use chrono::Utc;
use k0t1k_core::error::IoContext;
use k0t1k_core::install::{check_installed, extract_natives, install_game, reconstruct_assets};
use k0t1k_core::instance::{EffectiveSettings, Instance};
use k0t1k_core::java::ensure_java;
use k0t1k_core::launch::{Account, LaunchCommand, LaunchOptions, LogFn, LogStream, WindowSize, build_command, spawn_game};
use k0t1k_core::profile::LaunchProfile;
use k0t1k_core::progress::ProgressFn;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::settings::{GlobalSettings, OnGameLaunch};
use tauri::{AppHandle, State};
use tokio::process::Command;
use tokio::sync::watch;

use crate::error::{AppError, CommandResult, blocking};
use crate::events::{self, GameLog, InstanceStatePayload, LogLine, RunState, progress_fn, tracked};
use crate::state::AppState;
use crate::{cmdline, game, window};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    account_id: Option<String>,
) -> CommandResult<()> {
    let state = state.inner().clone();
    let manager = state.instances.clone();
    let id = instance_id.clone();
    let (instance, global) = blocking(move || Ok((manager.get(&id)?, GlobalSettings::load(manager.paths())?))).await?;

    let account = match &account_id {
        Some(id) => global.accounts.iter().find(|account| &account.id == id),
        None => global.selected_account(),
    }
    .ok_or_else(|| AppError::new("no account selected: add an offline account in the settings"))?;
    let account = Account::offline(&account.name);
    let settings = instance.effective_settings(&global);

    let kill_requests = state.reserve(&instance_id, &instance.name)?;
    events::instance_state(&app, &instance_id, RunState::Preparing, None);
    tauri::async_runtime::spawn(run_session(app, state, instance, settings, account, global.behavior.on_game_launch, kill_requests));
    Ok(())
}

#[tauri::command]
pub fn kill_instance(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    kill(&app, &state, &instance_id)
}

/// Stops the game, or cancels its preparation.
pub fn kill(app: &AppHandle, state: &AppState, instance_id: &str) -> CommandResult<()> {
    let mut running = state.running();
    let entry = running.get_mut(instance_id).ok_or_else(|| AppError::new("the instance is not running"))?;
    entry.cancel.send_replace(true);
    entry.state = RunState::Stopping;
    if let Some(handle) = &entry.handle {
        handle.kill();
    }
    events::instance_state(app, instance_id, RunState::Stopping, None);
    Ok(())
}

#[tauri::command]
pub fn list_run_states(state: State<'_, AppState>) -> Vec<InstanceStatePayload> {
    state
        .running()
        .iter()
        .map(|(id, entry)| InstanceStatePayload { instance_id: id.clone(), state: entry.state, exit_code: None })
        .collect()
}

/// Recent log lines of an instance, for a window opened after the game started.
#[tauri::command]
pub fn get_game_logs(state: State<'_, AppState>, instance_id: String) -> Vec<LogLine> {
    state.log_lines(&instance_id)
}

/// Clears the run entry and tells the UI the instance stopped when the session task ends without
/// [`SessionGuard::end`], i.e. when it panics; otherwise the instance would stay "running" forever.
struct SessionGuard {
    app: AppHandle,
    state: AppState,
    id: String,
    log: GameLog,
    ended: bool,
}

impl SessionGuard {
    /// Forgets the running game and tells the UI it stopped; playtime must be saved before this.
    fn end(&mut self, message: &str, exit_code: Option<i32>) {
        self.ended = true;
        self.log.line("launcher", message);
        self.state.running().remove(&self.id);
        events::instance_state(&self.app, &self.id, RunState::Stopped, exit_code);
        window::on_game_stopped(&self.app);
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

async fn run_session(
    app: AppHandle,
    state: AppState,
    instance: Instance,
    settings: EffectiveSettings,
    account: Account,
    on_game_launch: OnGameLaunch,
    mut kill_requests: watch::Receiver<bool>,
) {
    let id = instance.id.clone();
    let task = format!("launch:{id}");
    let title = format!("Запуск: {}", instance.name);
    let post_exit = settings.commands.post_exit.clone();
    let progress = progress_fn(&app, &task, &title);
    let log = GameLog::new(&app, &state, &id);
    let mut session = SessionGuard { app: app.clone(), state: state.clone(), id: id.clone(), log: log.clone(), ended: false };

    // Dropping `prepare` on a kill also stops its downloads and kills a running pre-launch hook.
    let preparation = async {
        tokio::select! {
            prepared = prepare(&state, &log, &instance, &settings, account, &task, &progress) => prepared,
            () = wait_killed(&mut kill_requests) => Err(AppError::new("запуск отменён")),
        }
    };
    let command = match tracked(&app, &task, &title, preparation).await {
        Ok(command) => command,
        Err(_) if state.kill_requested(&id) => return session.end("Запуск отменён", None),
        Err(error) => return session.end(&format!("Запуск не удался: {error}"), None),
    };
    if state.kill_requested(&id) {
        return session.end("Запуск отменён", None);
    }

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
    let killed_at_spawn = {
        let mut running = state.running();
        match running.get_mut(&id) {
            Some(entry) => {
                entry.handle = Some(handle.clone());
                if entry.kill_requested() {
                    handle.kill();
                } else {
                    entry.state = RunState::Running;
                    // Emitted under the lock so a concurrent kill's `Stopping` cannot be overwritten.
                    events::instance_state(&app, &id, RunState::Running, None);
                }
                entry.kill_requested()
            }
            None => false,
        }
    };

    if !killed_at_spawn {
        window::on_game_started(&app, on_game_launch);
    }

    let exit_code = match handle.wait().await {
        Ok(code) => code,
        Err(error) => {
            log.line("launcher", &format!("Не удалось дождаться завершения игры: {error}"));
            None
        }
    };
    let ended_at = Utc::now();

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
    let message = if state.kill_requested(&id) {
        "Игра была завершена принудительно".to_owned()
    } else {
        match exit_code {
            Some(code) => format!("Игра завершилась с кодом {code}"),
            None => "Игра была завершена сигналом".to_owned(),
        }
    };
    session.end(&message, exit_code);

    if !post_exit.is_empty() {
        let game_dir = instance.game_dir(&state.paths);
        if let Err(error) = run_hook("post-exit", &post_exit, &game_dir).await {
            log.line("launcher", &error.to_string());
        }
    }
}

async fn prepare(
    state: &AppState,
    log: &GameLog,
    instance: &Instance,
    settings: &EffectiveSettings,
    account: Account,
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
    let natives_dir = instance.natives_dir(paths);
    match tokio::fs::remove_dir_all(&natives_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => Err::<(), _>(error).at(&natives_dir)?,
    }
    extract_natives(paths, &profile, &natives_dir).await?;
    reconstruct_assets(paths, &assets, &game_dir).await?;

    if !settings.commands.pre_launch.is_empty() {
        run_hook("pre-launch", &settings.commands.pre_launch, &game_dir).await?;
    }

    let window = (!settings.window.maximized).then_some(WindowSize { width: settings.window.width, height: settings.window.height });
    let options = LaunchOptions {
        java_path: java.path,
        java_major: java.major,
        game_dir,
        natives_dir,
        profile_name: instance.name.clone(),
        min_memory_mb: settings.memory.min_mib,
        max_memory_mb: settings.memory.max_mib,
        extra_jvm_args: cmdline::split(&settings.jvm_args)?,
        window,
        account,
        env: settings.env.clone(),
        wrapper: cmdline::split(&settings.commands.wrapper)?,
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
