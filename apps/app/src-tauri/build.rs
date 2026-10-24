use std::path::Path;

fn main() {
    embed_from_env("K0T1K_CURSEFORGE_API_KEY", "CurseForge will be unavailable");
    embed_from_env("K0T1K_MSA_CLIENT_ID", "Microsoft sign-in will be unavailable");
    tauri_build::build()
}

/// Bakes a value (the CurseForge API key, the Azure application id) into the binary at build time,
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
