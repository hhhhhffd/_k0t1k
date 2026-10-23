//! Global launcher settings stored as `settings.json`.
//!
//! Every group has serde defaults, so older or hand-edited files keep loading when fields are added.
//! Instances override whole groups through `Option<T>` (see `instance::InstanceSettings`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

/// Serialises read-modify-write cycles of the settings file within this process.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JavaSettings {
    /// Explicit Java executable. Wins over any automatic choice.
    pub path: Option<PathBuf>,
    /// Pick a detected or managed Java that fits the game version.
    pub auto_select: bool,
    /// Download Mojang's runtime when nothing suitable is installed.
    pub auto_download: bool,
}

impl Default for JavaSettings {
    fn default() -> Self {
        Self {
            path: None,
            auto_select: true,
            auto_download: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MemorySettings {
    pub min_mib: u32,
    pub max_mib: u32,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            min_mib: 512,
            max_mib: default_max_memory_mib(total_memory_mib()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 854,
            height: 480,
            maximized: false,
        }
    }
}

/// Shell command lines; an empty string means "not set".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommandSettings {
    pub pre_launch: String,
    pub post_exit: String,
    pub wrapper: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaytimeSettings {
    pub record: bool,
    pub show: bool,
    /// Seconds played across all instances.
    pub total_secs: u64,
}

impl Default for PlaytimeSettings {
    fn default() -> Self {
        Self {
            record: true,
            show: true,
            total_secs: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiSettings {
    /// A built-in theme id or the id of one of `custom_themes`.
    pub theme: String,
    /// Lightweight mode for low-end PCs.
    pub lite_mode: bool,
    pub custom_themes: Vec<CustomTheme>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTheme {
    pub id: String,
    pub name: String,
    /// Built-in theme this one starts from.
    pub base: String,
    /// CSS variable name without the leading `--` mapped to a color value.
    pub colors: BTreeMap<String, String>,
}

impl UiSettings {
    /// Theme values are injected into CSS by the UI, so only a narrow charset is accepted.
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for theme in &self.custom_themes {
            if theme.id.is_empty() || !seen.insert(theme.id.as_str()) {
                return Err(Error::Other(format!("custom theme id {:?} is empty or not unique", theme.id)));
            }
            for (key, value) in &theme.colors {
                let key_ok = (1..=40).contains(&key.len())
                    && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                let value_ok = (1..=64).contains(&value.len())
                    && value.chars().all(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || "#(),.%-".contains(c));
                if !key_ok || !value_ok {
                    return Err(Error::Other(format!(
                        "invalid color {key:?}: {value:?} in custom theme {:?}",
                        theme.name
                    )));
                }
            }
        }
        Ok(())
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "k0t1k".to_owned(),
            lite_mode: false,
            custom_themes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GlobalSettings {
    pub java: JavaSettings,
    pub memory: MemorySettings,
    pub jvm_args: String,
    pub window: WindowSettings,
    pub commands: CommandSettings,
    pub env: BTreeMap<String, String>,
    pub playtime: PlaytimeSettings,
    pub ui: UiSettings,
    pub accounts: Vec<Account>,
    pub selected_account_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
}

impl GlobalSettings {
    pub fn selected_account(&self) -> Option<&Account> {
        let id = self.selected_account_id.as_deref()?;
        self.accounts.iter().find(|account| account.id == id)
    }
}

impl GlobalSettings {
    /// Load the settings, or the defaults when the file does not exist yet.
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.settings_file();
        match std::fs::read(&file) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Io { path: file, source }),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_json_atomic(&paths.settings_file(), self)
    }

    /// Load, apply `change` and save while holding the process-wide settings lock,
    /// so concurrent updates (e.g. playtime of two instances) do not overwrite each other.
    pub fn update(paths: &Paths, change: impl FnOnce(&mut Self)) -> Result<Self> {
        let _guard = UPDATE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut settings = Self::load(paths)?;
        change(&mut settings);
        settings.save(paths)?;
        Ok(settings)
    }
}

/// Prism's default max heap: two thirds of RAM below 6 GiB, otherwise 4 GiB.
pub fn default_max_memory_mib(total_mib: Option<u64>) -> u32 {
    match total_mib {
        Some(total) if total < 6144 => (total * 2 / 3) as u32,
        Some(_) => 4096,
        None => 2048,
    }
}

/// Total physical memory in MiB, `None` when the platform does not report it.
pub fn total_memory_mib() -> Option<u64> {
    total_memory_bytes().map(|bytes| bytes / (1024 * 1024))
}

#[cfg(windows)]
fn total_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `status` is a valid MEMORYSTATUSEX with `dwLength` set as the API requires.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.ullTotalPhys)
}

#[cfg(target_os = "linux")]
fn total_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo_total_kib(&meminfo).map(|kib| kib * 1024)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn total_memory_bytes() -> Option<u64> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn parse_meminfo_total_kib(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find_map(|line| line.strip_prefix("MemTotal:"))?;
    line.split_whitespace().next()?.parse().ok()
}

/// Write JSON next to `path` and rename it into place, so readers never see a half-written file.
pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).at(parent)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp).at(&tmp)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.sync_all().at(&tmp)?;
    drop(file);
    std::fs::rename(&tmp, path).at(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_prism() {
        let settings = GlobalSettings::default();
        assert!(settings.java.auto_select && settings.java.auto_download);
        assert_eq!(settings.java.path, None);
        assert_eq!(settings.memory.min_mib, 512);
        assert_eq!((settings.window.width, settings.window.height), (854, 480));
        assert!(!settings.window.maximized);
        assert_eq!(settings.ui.theme, "k0t1k");
        assert!(settings.playtime.record && settings.playtime.show);
    }

    fn theme(id: &str, colors: &[(&str, &str)]) -> CustomTheme {
        CustomTheme {
            id: id.to_owned(),
            name: id.to_owned(),
            base: "k0t1k".to_owned(),
            colors: colors.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        }
    }

    #[test]
    fn custom_theme_validation() {
        let mut ui = UiSettings {
            custom_themes: vec![theme("a", &[("bg-1", "#fff"), ("accent", "rgb(1, 2, 3)"), ("x", "hsl(10 20% 30%)")]), theme("b", &[])],
            ..UiSettings::default()
        };
        assert!(ui.validate().is_ok());

        ui.custom_themes[1].id = "a".to_owned();
        assert!(ui.validate().is_err());
        ui.custom_themes[1].id = "b".to_owned();
        for (key, value) in [("Bad", "#fff"), ("ok", "red;}body{x:y"), ("ok", "a\\b"), ("ok", ""), ("", "#fff")] {
            ui.custom_themes[1].colors = [(key.to_owned(), value.to_owned())].into();
            assert!(ui.validate().is_err(), "{key}: {value}");
        }
    }

    #[test]
    fn max_memory_formula() {
        assert_eq!(default_max_memory_mib(Some(3072)), 2048);
        assert_eq!(default_max_memory_mib(Some(6143)), 4095);
        assert_eq!(default_max_memory_mib(Some(6144)), 4096);
        assert_eq!(default_max_memory_mib(Some(65536)), 4096);
        assert_eq!(default_max_memory_mib(None), 2048);
    }

    #[test]
    fn partial_file_gets_defaults() {
        let settings: GlobalSettings =
            serde_json::from_str(r#"{"jvmArgs":"-Xss1M","window":{"width":1000},"ui":{"liteMode":true}}"#).unwrap();
        assert_eq!(settings.jvm_args, "-Xss1M");
        assert_eq!((settings.window.width, settings.window.height), (1000, 480));
        assert!(settings.ui.lite_mode);
        assert_eq!(settings.ui.theme, "k0t1k");
    }

    #[test]
    fn save_load_roundtrip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"));
        assert_eq!(GlobalSettings::load(&paths).unwrap(), GlobalSettings::default());

        let updated = GlobalSettings::update(&paths, |s| {
            s.accounts.push(Account {
                id: "a1".to_owned(),
                name: "Steve".to_owned(),
                kind: AccountKind::Offline,
            });
            s.selected_account_id = Some("a1".to_owned());
            s.env.insert("A".to_owned(), "1".to_owned());
        })
        .unwrap();
        assert_eq!(GlobalSettings::load(&paths).unwrap(), updated);
        assert!(!paths.settings_file().with_extension("json.tmp").exists());
    }

    #[test]
    fn broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        std::fs::write(paths.settings_file(), "{nope").unwrap();
        assert!(matches!(GlobalSettings::load(&paths), Err(Error::Json(_))));
    }

    #[test]
    fn meminfo_parsing() {
        let sample = "MemTotal:       16318132 kB\nMemFree:         1000 kB\n";
        assert_eq!(parse_meminfo_total_kib(sample), Some(16318132));
        assert_eq!(parse_meminfo_total_kib("MemFree: 1 kB"), None);
    }
}
