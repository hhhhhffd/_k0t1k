use std::path::Path;
use std::process::Command;

fn main() {
    embed_from_env("K0T1K_CURSEFORGE_API_KEY", "CurseForge will be unavailable");
    embed_from_env("K0T1K_MSA_CLIENT_ID", "Microsoft sign-in will be unavailable");
    embed_from_env("K0T1K_DISCORD_CLIENT_ID", "Discord Rich Presence will be off");
    build_overlay();
    tauri_build::build()
}

/// Builds the overlay agent jar with Gradle (JDK 21 needed) and hands its path and version to
/// `include_bytes!`. Cargo reruns this only when the agent's sources change.
fn build_overlay() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../packages/overlay-agent");
    for input in ["src", "build.gradle.kts", "settings.gradle.kts"] {
        println!("cargo:rerun-if-changed={}", project.join(input).display());
    }
    let script = std::fs::read_to_string(project.join("build.gradle.kts")).expect("packages/overlay-agent/build.gradle.kts is missing");
    let version = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("version = ")?.trim().strip_prefix('"')?.strip_suffix('"').map(str::to_owned))
        .expect("build.gradle.kts has no `version = \"...\"` line");

    let mut gradle = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "gradlew.bat"]);
        cmd
    } else {
        Command::new("./gradlew")
    };
    // No daemon: a lingering Gradle JVM would look like a running game to anyone counting java processes.
    let status = gradle
        .args(["shadowJar", "--no-daemon", "--quiet"])
        .current_dir(&project)
        .status()
        .expect("cannot start Gradle for the overlay agent");
    assert!(status.success(), "building the overlay agent failed (it needs JDK 21): {status}");

    let jar = project.join(format!("build/libs/k0t1k-overlay-{version}.jar"));
    assert!(jar.is_file(), "Gradle did not produce {}", jar.display());
    println!("cargo:rustc-env=K0T1K_OVERLAY_VERSION={version}");
    println!("cargo:rustc-env=K0T1K_OVERLAY_JAR={}", jar.canonicalize().expect("overlay jar path").display());
}

/// Bakes a value (the CurseForge API key, the Azure and Discord application ids) into the binary at build time,
/// taken from the environment or from the gitignored `.env` at the repository root, so it never lives
/// in source control. The file is parsed directly rather than through a shell because CurseForge keys
/// start with `$2a$10$`.
fn embed_from_env(var: &str, missing: &str) {
    let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.env");
    println!("cargo:rerun-if-env-changed={var}");
    println!("cargo:rerun-if-changed={}", env_file.display());

    let value = std::env::var(var).ok().or_else(|| {
        let contents = std::fs::read_to_string(&env_file).ok()?;
        contents.lines().find_map(|line| {
            let value = line.trim().strip_prefix(var)?.strip_prefix('=')?;
            Some(value.trim().trim_matches(['"', '\'']).to_owned())
        })
    });

    match value.filter(|value| !value.is_empty()) {
        Some(value) => println!("cargo:rustc-env={var}={value}"),
        None => println!("cargo:warning={var} is not set; {missing}"),
    }
}
