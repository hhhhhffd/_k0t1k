//! Where the launcher keeps its data: next to the executable (portable, like Prism), in a folder
//! chosen through `data-dir.txt`, or in the user profile when the executable's folder is read-only.

use std::path::{Path, PathBuf};

use k0t1k_core::paths::Paths;
use serde::Serialize;

use crate::error::{AppError, CommandResult};

/// Holds the data directory chosen by the user; read at startup, next to the executable.
const OVERRIDE_FILE: &str = "data-dir.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataDirSource {
    /// The folder named in `data-dir.txt`.
    Custom,
    /// The executable's own folder.
    Portable,
    /// The user profile: the executable's folder is not writable, or `data-dir.txt` is unusable.
    Fallback,
}

#[derive(Debug, Clone)]
pub struct DataLocation {
    pub root: PathBuf,
    pub exe_dir: PathBuf,
    pub source: DataDirSource,
}

impl DataLocation {
    pub fn portable(&self) -> bool {
        self.source != DataDirSource::Fallback
    }

    /// WebView2 keeps cookies and caches here instead of in `%LOCALAPPDATA%`.
    pub fn webview_dir(&self) -> PathBuf {
        self.root.join("webview")
    }
}

/// Fails when `dir` cannot be created or written to.
pub fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".k0t1k-write-test-{}", std::process::id()));
    std::fs::File::create(&probe)?;
    std::fs::remove_file(&probe)
}

pub fn executable_dir() -> CommandResult<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| AppError::new("cannot determine the executable's directory"))
}

pub fn override_file(exe_dir: &Path) -> PathBuf {
    exe_dir.join(OVERRIDE_FILE)
}

pub fn resolve() -> CommandResult<DataLocation> {
    let exe_dir = executable_dir()?;
    let custom = match std::fs::read_to_string(override_file(&exe_dir)) {
        Ok(text) => Some(PathBuf::from(text.trim())).filter(|path| !path.as_os_str().is_empty()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(AppError::new(format!("cannot read {OVERRIDE_FILE}: {error}"))),
    };

    if let Some(root) = custom {
        match check_custom(&root) {
            Ok(()) => return Ok(DataLocation { root, exe_dir, source: DataDirSource::Custom }),
            Err(error) => tracing::warn!("ignoring {OVERRIDE_FILE}: {error}"),
        }
    } else if let Err(error) = ensure_writable(&exe_dir) {
        tracing::warn!("the executable's directory is not writable, using the user profile: {error}");
    } else {
        return Ok(DataLocation { root: exe_dir.clone(), exe_dir, source: DataDirSource::Portable });
    }

    let root = Paths::default_root().ok_or_else(|| AppError::new("cannot determine the data directory"))?;
    Ok(DataLocation { root, exe_dir, source: DataDirSource::Fallback })
}

/// A custom folder must be absolute: a relative one would depend on the working directory.
pub fn check_custom(path: &Path) -> CommandResult<()> {
    if !path.is_absolute() {
        return Err(AppError::new(format!("the data directory must be an absolute path: {}", path.display())));
    }
    ensure_writable(path).map_err(|error| AppError::new(format!("the data directory {} is not writable: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_directory_must_be_absolute_and_writable() {
        assert!(check_custom(Path::new("relative/dir")).is_err());
        let dir = std::env::temp_dir().join(format!("k0t1k-datadir-test-{}", std::process::id()));
        assert!(check_custom(&dir.join("nested")).is_ok());
        assert!(std::fs::read_dir(dir.join("nested")).unwrap().next().is_none(), "the probe file must be removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
