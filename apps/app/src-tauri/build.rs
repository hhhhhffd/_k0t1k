use std::path::Path;

const CURSEFORGE_KEY_VAR: &str = "K0T1K_CURSEFORGE_API_KEY";
const DISCORD_CLIENT_ID_VAR: &str = "K0T1K_DISCORD_CLIENT_ID";

fn main() {
    embed_curseforge_key();
    embed_discord_client_id();
    tauri_build::build()
}

/// Bakes the CurseForge API key into the binary at build time, taken from the environment or from the
/// gitignored `.env` at the repository root, so the key never lives in source control.
/// The file is parsed directly rather than through a shell because CurseForge keys start with `$2a$10$`.
fn embed_curseforge_key() {
    match build_value(CURSEFORGE_KEY_VAR) {
        Some(key) => println!("cargo:rustc-env={CURSEFORGE_KEY_VAR}={key}"),
        None => println!("cargo:warning={CURSEFORGE_KEY_VAR} is not set; CurseForge will be unavailable"),
    }
}

/// Bakes the Discord application (client) id for Rich Presence into the binary, like the CurseForge key.
fn embed_discord_client_id() {
    match build_value(DISCORD_CLIENT_ID_VAR) {
        Some(id) => println!("cargo:rustc-env={DISCORD_CLIENT_ID_VAR}={id}"),
        None => println!("cargo:warning={DISCORD_CLIENT_ID_VAR} is not set; Discord Rich Presence will be off"),
    }
}

/// The variable from the environment, else from the `.env` at the repository root; empty counts as unset.
fn build_value(name: &str) -> Option<String> {
    let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.env");
    println!("cargo:rerun-if-env-changed={name}");
    println!("cargo:rerun-if-changed={}", env_file.display());

    std::env::var(name)
        .ok()
        .or_else(|| {
            let contents = std::fs::read_to_string(&env_file).ok()?;
            contents.lines().find_map(|line| {
                let value = line.trim().strip_prefix(name)?.strip_prefix('=')?;
                Some(value.trim().trim_matches(['"', '\'']).to_owned())
            })
        })
        .filter(|value| !value.is_empty())
}
