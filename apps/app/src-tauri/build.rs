use std::path::Path;

const CURSEFORGE_KEY_VAR: &str = "K0T1K_CURSEFORGE_API_KEY";

fn main() {
    embed_curseforge_key();
    tauri_build::build()
}

/// Bakes the CurseForge API key into the binary at build time, taken from the environment or from the
/// gitignored `.env` at the repository root, so the key never lives in source control.
/// The file is parsed directly rather than through a shell because CurseForge keys start with `$2a$10$`.
fn embed_curseforge_key() {
    let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.env");
    println!("cargo:rerun-if-env-changed={CURSEFORGE_KEY_VAR}");
    println!("cargo:rerun-if-changed={}", env_file.display());

    let key = std::env::var(CURSEFORGE_KEY_VAR).ok().or_else(|| {
        let contents = std::fs::read_to_string(&env_file).ok()?;
        contents.lines().find_map(|line| {
            let value = line.trim().strip_prefix(CURSEFORGE_KEY_VAR)?.strip_prefix('=')?;
            Some(value.trim().trim_matches(['"', '\'']).to_owned())
        })
    });

    match key.filter(|key| !key.is_empty()) {
        Some(key) => println!("cargo:rustc-env={CURSEFORGE_KEY_VAR}={key}"),
        None => println!("cargo:warning={CURSEFORGE_KEY_VAR} is not set; CurseForge will be unavailable"),
    }
}
