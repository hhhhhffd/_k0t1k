//! The in-game overlay agent (repository `hhhhhffd/_k0t1k-overlay`): placing its jar and the JVM arguments
//! that load it. The jar comes from the plugin system: a baseline in the launcher binary or a signed release. The JVM keeps an agent jar open (locked on Windows)
//! for the whole session, so each version gets its own file name and old ones are removed only while no
//! game runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::i18n::Language;

const JAR_PREFIX: &str = "k0t1k-overlay-";
const HISTORY_FILE: &str = "chat-history.tsv";
const THEME_FILE: &str = "theme.json";
const THEME_VERSION: u32 = 1;
/// The colour tokens of the launcher UI (`--c-*` in style.css, without the prefix) that the overlay reads.
const THEME_TOKENS: [&str; 14] =
    ["base", "surface", "raised", "hover", "line", "fg", "muted", "faint", "accent", "accent-fg", "ok", "warn", "danger", "icon"];

/// The launcher's resolved theme as the overlay reads it from `<dataDir>/theme.json`. The UI resolves
/// built-in, custom and file themes into concrete colours (it owns the CSS), so the contract is the
/// computed result and no copy of the built-in palettes is kept here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayTheme {
    pub id: String,
    pub name: String,
    /// Whether the theme is dark (its `color-scheme`).
    pub dark: bool,
    /// Token name without `--c-` mapped to `#rrggbb`.
    pub colors: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct ThemeFile<'a> {
    version: u32,
    #[serde(flatten)]
    theme: &'a OverlayTheme,
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Writes the theme for the overlay (atomically, since a running game polls the file). The overlay
/// copes with missing tokens, but a malformed colour means a bug in the caller, so it is refused.
pub fn write_theme(data_dir: &Path, theme: &OverlayTheme) -> Result<()> {
    for (token, value) in &theme.colors {
        if !THEME_TOKENS.contains(&token.as_str()) || !is_hex_color(value) {
            return Err(Error::Other(format!("invalid overlay theme color {token:?}: {value:?}")));
        }
    }
    let json = serde_json::to_vec_pretty(&ThemeFile { version: THEME_VERSION, theme }).map_err(|error| Error::Other(error.to_string()))?;
    std::fs::create_dir_all(data_dir).at(data_dir)?;
    let path = data_dir.join(THEME_FILE);
    let partial = data_dir.join(format!("{THEME_FILE}.part"));
    std::fs::write(&partial, json).at(&partial)?;
    std::fs::rename(&partial, &path).at(&path)?;
    Ok(())
}

fn jar_path(dir: &Path, version: &str) -> PathBuf {
    dir.join(format!("{JAR_PREFIX}{version}.jar"))
}

/// Writes the jar of `version` into `dir` unless an identical copy is there. Returns its path.
pub fn install_jar(dir: &Path, version: &str, bytes: &[u8]) -> Result<PathBuf> {
    let path = jar_path(dir, version);
    if std::fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).at(dir)?;
    let partial = dir.join(format!("{JAR_PREFIX}{version}.jar.part"));
    std::fs::write(&partial, bytes).at(&partial)?;
    std::fs::rename(&partial, &path).at(&path)?;
    Ok(path)
}

/// Deletes the jars of other versions. Call only while no game runs: a running game locks its jar.
pub fn remove_old_jars(dir: &Path, keep_version: &str) -> Result<()> {
    let keep = jar_path(dir, keep_version);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).at(dir),
    };
    for entry in entries {
        let path = entry.at(dir)?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with(JAR_PREFIX) && name.ends_with(".jar") && path != keep {
            std::fs::remove_file(&path).at(&path)?;
        }
    }
    Ok(())
}

/// The agent hooks LWJGL 3 (Minecraft 1.13 on) and LWJGL 2 (1.8 to 1.12.2, tested on 1.8.9 and 1.12.2).
/// Older versions are left out: 1.7 and before were not tried and some run on Java 7, which cannot load the agent.
pub fn supports(minecraft_version: &str) -> bool {
    let number = |text: &str| text.split(['.', '-', ' ']).next().and_then(|part| part.parse::<u32>().ok());
    if let Some(rest) = minecraft_version.strip_prefix("1.") {
        return number(rest).is_some_and(|minor| minor >= 8);
    }
    // weekly snapshots, `18w01a`: 1.9 snapshots (`15w31a`) and later
    if let Some((year, _)) = minecraft_version.split_once('w') {
        return year.parse::<u32>().is_ok_and(|year| year >= 15);
    }
    // year-based versions such as 26.2 or 26.3-snapshot-1
    number(minecraft_version).is_some_and(|major| major >= 25)
}

/// JVM arguments that load the agent. The launcher forces `user.language`, so the UI language is passed.
/// `scale` is the interface zoom in percent; without it the overlay picks one from the screen size.
pub fn agent_args(jar: &Path, data_dir: &Path, language: Language, scale: Option<u16>) -> Vec<String> {
    let lang = match language {
        Language::Ru => "ru",
        Language::En => "en",
    };
    let scale = scale.map_or_else(String::new, |percent| format!(",scale={percent}"));
    vec![format!("-javaagent:{}=lang={lang}{scale}", jar.display()), format!("-Dk0t1k.overlay.dataDir={}", data_dir.display())]
}

/// Deletes the saved chat history. Call only while no game runs.
pub fn clear_history(data_dir: &Path) -> Result<()> {
    let path = data_dir.join(HISTORY_FILE);
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error).at(&path),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jars_are_versioned_and_old_ones_removed() {
        let dir = tempfile::tempdir().unwrap();
        let first = install_jar(dir.path(), "0.1.0", b"one").unwrap();
        assert!(first.ends_with("k0t1k-overlay-0.1.0.jar"));
        let second = install_jar(dir.path(), "0.2.0", b"two").unwrap();
        // the same version again is left alone
        assert_eq!(install_jar(dir.path(), "0.2.0", b"two").unwrap(), second);
        std::fs::write(dir.path().join("chat-history.tsv"), "x").unwrap();

        remove_old_jars(dir.path(), "0.2.0").unwrap();
        assert!(!first.exists());
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert!(dir.path().join("chat-history.tsv").exists());
    }

    #[test]
    fn theme_is_written_atomically_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let colors = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut theme = OverlayTheme { id: "light".into(), name: "Light".into(), dark: false, colors: colors(&[("base", "#f1eff7"), ("accent-fg", "#ffffff")]) };
        write_theme(dir.path(), &theme).unwrap();
        let path = dir.path().join("theme.json");
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["version"], 1);
        assert_eq!(written["id"], "light");
        assert_eq!(written["dark"], false);
        assert_eq!(written["colors"]["accent-fg"], "#ffffff");
        assert!(!dir.path().join("theme.json.part").exists());

        let before = std::fs::read(&path).unwrap();
        for (token, value) in [("base", "red"), ("base", "#fff"), ("base", "#12345g"), ("bogus", "#000000")] {
            theme.colors = colors(&[(token, value)]);
            assert!(write_theme(dir.path(), &theme).is_err(), "{token} {value}");
        }
        // a refused theme leaves the previous file alone
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn supported_minecraft_versions() {
        for version in ["1.8", "1.8.9", "1.12.2", "1.13", "1.16.5", "1.21.11", "26.2", "26.3-snapshot-1", "24w14a", "15w31a"] {
            assert!(supports(version), "{version}");
        }
        for version in ["1.7.10", "1.6.4", "b1.7.3", "rd-132211", "14w34d", ""] {
            assert!(!supports(version), "{version}");
        }
    }

    #[test]
    fn arguments_carry_language_and_data_dir() {
        let args = agent_args(Path::new("/d/k0t1k-overlay-0.1.0.jar"), Path::new("/d/my data"), Language::Ru, None);
        assert_eq!(args, ["-javaagent:/d/k0t1k-overlay-0.1.0.jar=lang=ru", "-Dk0t1k.overlay.dataDir=/d/my data"]);
    }

    #[test]
    fn arguments_carry_the_interface_zoom() {
        let args = agent_args(Path::new("/x.jar"), Path::new("/d"), Language::Ru, Some(125));
        assert_eq!(args[0], "-javaagent:/x.jar=lang=ru,scale=125");
    }

    #[test]
    fn clearing_history_without_one_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        clear_history(dir.path()).unwrap();
        std::fs::write(dir.path().join("chat-history.tsv"), "x").unwrap();
        clear_history(dir.path()).unwrap();
        assert!(!dir.path().join("chat-history.tsv").exists());
    }
}
