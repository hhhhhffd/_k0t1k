//! Installs and launches a game for manual end-to-end checks, then kills it after a while.
//!
//! ```text
//! cargo run -p k0t1k-core --example launch -- <data dir> <java> <java major> <mc version> [<loader uid> <loader version>]
//! ```
//! Environment: `K0T1K_SECONDS` (default 30) is how long the game runs, `K0T1K_JAVA_ARCH` (default
//! `amd64`) the `os.arch` of the Java binary.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use k0t1k_core::component::ComponentSpec;
use k0t1k_core::install::{extract_natives, install_game, reconstruct_assets};
use k0t1k_core::launch::{Account, LaunchOptions, LogFn, WindowSize, build_command, spawn_game};
use k0t1k_core::meta::MetaClient;
use k0t1k_core::paths::Paths;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::profile::{LaunchProfile, resolve_components};
use k0t1k_core::progress::{ProgressEvent, ProgressFn};
use k0t1k_core::{Error, Result};

fn spec(uid: &str, version: &str) -> ComponentSpec {
    ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data_dir, java, java_major, mc_version, loader @ ..] = args.as_slice() else {
        return Err(Error::Other(
            "usage: launch <data dir> <java> <java major> <mc version> [<loader uid> <loader version>]".to_owned(),
        ));
    };
    let java_major: u32 = java_major.parse().map_err(|_| Error::Other("java major must be a number".to_owned()))?;
    let seconds: u64 = std::env::var("K0T1K_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(30);
    let java_arch = std::env::var("K0T1K_JAVA_ARCH").unwrap_or_else(|_| "amd64".to_owned());

    let paths = Paths::new(data_dir);
    let meta = MetaClient::new(&paths, false);
    let mut specs = vec![spec("net.minecraft", mc_version)];
    if let [uid, version] = loader {
        specs.push(spec(uid, version));
    }
    let components = resolve_components(&meta, specs).await?;
    for component in &components {
        println!(
            "component {} {} (dependency only: {})",
            component.spec.uid, component.spec.version, component.spec.dependency_only
        );
    }
    let profile = LaunchProfile::build(&components, RuntimeContext::host(&java_arch))?;
    if !profile.is_java_compatible(java_major) {
        return Err(Error::Other(format!("Java {java_major} is not in {:?}", profile.compatible_java_majors)));
    }

    let progress: ProgressFn = Arc::new(|event: ProgressEvent| {
        if event.done == event.total || event.done.is_multiple_of(250) {
            println!("[{}] {}/{}", event.stage, event.done, event.total);
        }
    });
    let client = k0t1k_core::download::http_client();
    let assets = install_game(&client, &paths, &profile, "e2e", &progress).await?;

    let instance_dir = paths.instances_dir().join("e2e");
    let game_dir = instance_dir.join("minecraft");
    let natives_dir = instance_dir.join("natives");
    tokio::fs::create_dir_all(&game_dir).await.map_err(|source| Error::Io { path: game_dir.clone(), source })?;
    extract_natives(&paths, &profile, &natives_dir).await?;
    reconstruct_assets(&paths, &assets, &game_dir).await?;

    let options = LaunchOptions {
        java_path: PathBuf::from(java),
        java_major,
        game_dir,
        natives_dir,
        profile_name: "e2e".to_owned(),
        min_memory_mb: 512,
        max_memory_mb: 2048,
        java_agent_args: Vec::new(),
        extra_jvm_args: Vec::new(),
        window: Some(WindowSize { width: 854, height: 480 }),
        account: Account::offline("Player"),
        env: BTreeMap::new(),
        wrapper: Vec::new(),
    };
    let command = build_command(&paths, &profile, &assets, &options).await?;
    println!("command: {}", command.display_redacted());

    let on_log: LogFn = Arc::new(|line| println!("game: {}", line.line));
    let handle = spawn_game(&command, on_log)?;
    println!("started pid {:?}, killing after {seconds}s", handle.pid());

    let killer = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        killer.kill();
    });
    println!("exit code: {:?}", handle.wait().await?);
    Ok(())
}
