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
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::settings::GlobalSettings;
use tauri::{AppHandle, State};
use tokio::process::Command;

use crate::error::{AppError, CommandResult, blocking};
use crate::events::{self, InstanceStatePayload, RunState, progress_fn, tracked};
use crate::state::AppState;
use crate::{cmdline, game};

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

    state.reserve(&instance_id)?;
    events::instance_state(&app, &instance_id, RunState::Preparing, None);
    tauri::async_runtime::spawn(run_session(app, state, instance, settings, account));
    Ok(())
}

#[tauri::command]
pub fn kill_instance(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    let mut running = state.running();
    let entry = running.get_mut(&instance_id).ok_or_else(|| AppError::new("the instance is not running"))?;
    entry.kill_requested = true;
    entry.state = RunState::Stopping;
    if let Some(handle) = &entry.handle {
        handle.kill();
    }
    events::instance_state(&app, &instance_id, RunState::Stopping, None);
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

async fn run_session(app: AppHandle, state: AppState, instance: Instance, settings: EffectiveSettings, account: Account) {
    let id = instance.id.clone();
    let task = format!("launch:{id}");
    let title = format!("Запуск: {}", instance.name);
    let post_exit = settings.commands.post_exit.clone();

    let prepared = tracked(&app, &task, &title, prepare(&app, &state, &instance, &settings, account, &task, &title)).await;
    let command = match prepared {
        Ok(command) => command,
        Err(error) => return end_session(&app, &state, &id, &format!("Запуск не удался: {error}"), None),
    };
    if state.running().get(&id).is_some_and(|entry| entry.kill_requested) {
        return end_session(&app, &state, &id, "Запуск отменён", None);
    }

    let on_log: LogFn = {
        let (app, id) = (app.clone(), id.clone());
        Arc::new(move |line| {
            let stream = match line.stream {
                LogStream::Stdout => "stdout",
                LogStream::Stderr => "stderr",
            };
            events::game_log(&app, &id, stream, &line.line);
        })
    };
    let handle = match spawn_game(&command, on_log) {
        Ok(handle) => handle,
        Err(error) => return end_session(&app, &state, &id, &format!("Не удалось запустить игру: {}", AppError::from(error)), None),
    };
    let started_at = Utc::now();
    let kill_requested = {
        let mut running = state.running();
        match running.get_mut(&id) {
            Some(entry) => {
                entry.handle = Some(handle.clone());
                if entry.kill_requested {
                    handle.kill();
                } else {
                    entry.state = RunState::Running;
                }
                entry.kill_requested
            }
            None => false,
        }
    };
    if !kill_requested {
        events::instance_state(&app, &id, RunState::Running, None);
    }

    let exit_code = match handle.wait().await {
        Ok(code) => code,
        Err(error) => {
            events::game_log(&app, &id, "launcher", &format!("Не удалось дождаться завершения игры: {error}"));
            None
        }
    };
    let ended_at = Utc::now();

    let manager = state.instances.clone();
    let recorded = {
        let id = id.clone();
        blocking(move || manager.record_session(&id, started_at, ended_at).map(|_| ())).await
    };
    if let Err(error) = recorded {
        events::game_log(&app, &id, "launcher", &format!("Не удалось сохранить время игры: {error}"));
    }
    let message = match exit_code {
        Some(code) => format!("Игра завершилась с кодом {code}"),
        None => "Игра была завершена принудительно".to_owned(),
    };
    end_session(&app, &state, &id, &message, exit_code);

    if !post_exit.is_empty() {
        let game_dir = instance.game_dir(&state.paths);
        if let Err(error) = run_hook("post-exit", &post_exit, &game_dir).await {
            events::game_log(&app, &id, "launcher", &error.to_string());
        }
    }
}

/// Forgets the running game and tells the UI it stopped; playtime must be saved before this.
fn end_session(app: &AppHandle, state: &AppState, instance_id: &str, message: &str, exit_code: Option<i32>) {
    events::game_log(app, instance_id, "launcher", message);
    state.running().remove(instance_id);
    events::instance_state(app, instance_id, RunState::Stopped, exit_code);
}

async fn prepare(
    app: &AppHandle,
    state: &AppState,
    instance: &Instance,
    settings: &EffectiveSettings,
    account: Account,
    task: &str,
    title: &str,
) -> CommandResult<LaunchCommand> {
    let paths = &state.paths;
    let id = &instance.id;
    let progress = progress_fn(app, task, title);

    let (components, online) = game::resolve(paths, instance.components.clone()).await?;
    // Only the Java requirements are needed from this profile; natives depend on the chosen Java.
    let preliminary = LaunchProfile::build(&components, RuntimeContext::host(game::host_java_arch()))?;
    let java = ensure_java(
        paths,
        &state.http,
        &preliminary.compatible_java_majors,
        preliminary.compatible_java_name.as_deref(),
        &settings.java,
        &progress,
    )
    .await?;
    if !preliminary.is_java_compatible(java.major) {
        let warning = format!(
            "Предупреждение: Java {} может не подходить этой версии игры (нужна одна из {:?})",
            java.major, preliminary.compatible_java_majors
        );
        events::game_log(app, id, "launcher", &warning);
    }

    let profile = LaunchProfile::build(&components, RuntimeContext::host(&java.arch))?;
    let assets = if online {
        install_game(&state.http, paths, &profile, task, &progress).await?
    } else {
        events::game_log(app, id, "launcher", "Нет сети: используются уже загруженные файлы игры");
        check_installed(paths, &profile).await?
    };

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
    events::game_log(app, id, "launcher", &format!("Команда запуска: {}", command.display_redacted()));
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
        .stderr(Stdio::null());
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
