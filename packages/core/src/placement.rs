//! Where the launcher may live. A downloaded executable usually sits in Desktop or Downloads, and in
//! portable mode its data would be spread over that folder; this module holds the pure rules the app
//! uses to notice that, to pick an install folder and to make sure an uninstall never reaches
//! a folder that belongs to the user.

use std::path::{Path, PathBuf};

use crate::error::{Error, IoContext, Result};

/// Name of the folder created inside a chosen folder that is not empty.
pub const INSTALL_SUBFOLDER: &str = "_k0t1k";

/// Entries of a data directory (or of the executable's folder) that the launcher itself creates.
/// Only these are ever moved or deleted: a folder shared with other files keeps them.
pub const OWNED_NAMES: &[&str] = &[
    "instances",
    "java",
    "libraries",
    "assets",
    "meta",
    "shared",
    "cache",
    "natives",
    "webview",
    "settings.json",
    "settings.json.tmp",
    "data-dir.txt",
];

/// Folders of the current user that placement depends on; `detect` asks the OS (on Windows the
/// Known Folder API, so a OneDrive-redirected Desktop is found) instead of guessing paths.
#[derive(Debug, Clone)]
pub struct KnownFolders {
    pub desktop: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub profile: Option<PathBuf>,
    pub temp: PathBuf,
}

impl KnownFolders {
    pub fn detect() -> Self {
        Self {
            desktop: dirs::desktop_dir(),
            downloads: dirs::download_dir(),
            documents: dirs::document_dir(),
            profile: dirs::home_dir(),
            temp: std::env::temp_dir(),
        }
    }

    /// Folders where a launcher that keeps its data next to itself would make a mess.
    fn cluttered(&self) -> impl Iterator<Item = &Path> {
        [&self.desktop, &self.downloads, &self.documents]
            .into_iter()
            .flatten()
            .map(PathBuf::as_path)
    }

    /// Folders that must never be deleted, nor any folder above them.
    fn protected(&self) -> impl Iterator<Item = &Path> {
        self.cluttered()
            .chain(self.profile.as_deref())
            .chain(Some(self.temp.as_path()))
    }
}

/// Why the folder of the executable is a poor home for the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clutter {
    /// It is the Desktop, Downloads or Documents folder itself.
    UserFolder,
    /// It is inside the temp folder, e.g. the executable was started straight from a zip archive.
    Temp,
}

pub fn classify(exe_dir: &Path, folders: &KnownFolders) -> Option<Clutter> {
    let dir = normalize(exe_dir);
    if dir.starts_with(normalize(&folders.temp)) {
        Some(Clutter::Temp)
    } else if folders.cluttered().any(|folder| dir == normalize(folder)) {
        Some(Clutter::UserFolder)
    } else {
        None
    }
}

/// Whether both paths name the same folder (case-insensitively, links resolved when they exist).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// Canonical, lowercase and without the `\\?\` prefix, so paths from different sources compare equal.
fn normalize(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().to_lowercase();
    let plain = text
        .strip_prefix(r"\\?\")
        .filter(|rest| !rest.starts_with("unc\\"))
        .unwrap_or(&text);
    PathBuf::from(plain)
}

/// The folder to install into when the user picked `chosen`: the folder itself when it is empty,
/// missing or already holds a launcher, otherwise a `_k0t1k` folder inside it.
pub fn install_target(chosen: &Path, exe_name: &str) -> Result<PathBuf> {
    if !chosen.is_absolute() {
        return Err(Error::Other(format!(
            "the install folder must be an absolute path: {}",
            chosen.display()
        )));
    }
    if is_usable(chosen, exe_name)? {
        return Ok(chosen.to_path_buf());
    }
    let nested = chosen.join(INSTALL_SUBFOLDER);
    if is_usable(&nested, exe_name)? {
        Ok(nested)
    } else {
        Err(Error::Other(format!(
            "{} already exists and is not empty",
            nested.display()
        )))
    }
}

fn is_usable(dir: &Path, exe_name: &str) -> Result<bool> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none()
            || dir.join(exe_name).exists()
            || dir.join("settings.json").exists()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).at(dir),
    }
}

/// Entries of `dir` that the launcher created (see [`OWNED_NAMES`]).
pub fn owned_entries(dir: &Path) -> Vec<PathBuf> {
    OWNED_NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.symlink_metadata().is_ok())
        .collect()
}

/// Everything an uninstall removes. Built by [`RemovalPlan::new`], which refuses folders that must
/// never be deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPlan {
    /// Folders the launcher lives in: the data directory and the executable's folder (they may be one).
    /// They are removed only when nothing else is left in them.
    pub folders: Vec<PathBuf>,
    /// The launcher's own folders and files inside `folders`, and the executable.
    pub entries: Vec<PathBuf>,
    /// Shortcuts the launcher created.
    pub shortcuts: Vec<PathBuf>,
}

impl RemovalPlan {
    pub fn new(
        data_root: &Path,
        exe: &Path,
        shortcuts: &[PathBuf],
        known: &KnownFolders,
    ) -> Result<Self> {
        let exe_dir = exe
            .parent()
            .ok_or_else(|| Error::Other(format!("{} has no folder", exe.display())))?;
        let mut folders = vec![data_root.to_path_buf()];
        if !same_dir(data_root, exe_dir) {
            folders.push(exe_dir.to_path_buf());
        }
        for folder in &folders {
            check_removable(folder, known)?;
        }
        let mut entries: Vec<PathBuf> = folders
            .iter()
            .flat_map(|folder| owned_entries(folder))
            .collect();
        entries.push(exe.to_path_buf());
        let shortcuts = shortcuts
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect();
        Ok(Self {
            folders,
            entries,
            shortcuts,
        })
    }

    /// A `cmd /c` script that waits for the launcher to exit, deletes everything and tries once more
    /// after a pause (antivirus and WebView2 may hold files for a moment). Paths are quoted; `%` is
    /// refused because `cmd` expands it even inside quotes.
    pub fn cmd_script(&self) -> Result<String> {
        let all = self
            .folders
            .iter()
            .chain(&self.entries)
            .chain(&self.shortcuts);
        for path in all {
            let text = path.to_string_lossy();
            if text.contains(['%', '"']) {
                return Err(Error::Other(format!(
                    "cannot uninstall: the path {text} contains a % or a quote"
                )));
            }
        }
        let mut deletions = Vec::new();
        for path in self.entries.iter().chain(&self.shortcuts) {
            let quoted = format!("\"{}\"", path.display());
            // `del` fails on a folder and `rmdir` on a file, so each command is a no-op for the other kind.
            deletions.push(format!("del /f /q {quoted} 2>nul"));
            deletions.push(format!("rmdir /s /q {quoted} 2>nul"));
        }
        for folder in &self.folders {
            deletions.push(format!("rmdir \"{}\" 2>nul", folder.display()));
        }
        let pass = deletions.join(" & ");
        Ok(format!(
            "ping -n 4 127.0.0.1 >nul & {pass} & ping -n 4 127.0.0.1 >nul & {pass}"
        ))
    }
}

/// Refuses a drive root and any folder that is, or contains, the user's profile, Desktop,
/// Downloads, Documents or temp folder.
pub fn check_removable(dir: &Path, known: &KnownFolders) -> Result<()> {
    if !dir.is_absolute() || dir.parent().is_none() {
        return Err(Error::Other(format!(
            "refusing to remove {}: not a dedicated folder",
            dir.display()
        )));
    }
    let candidate = normalize(dir);
    if known
        .protected()
        .any(|folder| normalize(folder).starts_with(&candidate))
    {
        return Err(Error::Other(format!(
            "refusing to remove {}: it is, or contains, a folder of yours (Desktop, Downloads, Documents, profile or temp)",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows paths are only absolute, and `\` only separates components, on Windows.
    #[cfg(windows)]
    mod windows_paths {
        use super::*;

        fn folders() -> KnownFolders {
            KnownFolders {
                desktop: Some(PathBuf::from(r"C:\Users\Ann\OneDrive\Desktop")),
                downloads: Some(PathBuf::from(r"C:\Users\Ann\Downloads")),
                documents: Some(PathBuf::from(r"C:\Users\Ann\Documents")),
                profile: Some(PathBuf::from(r"C:\Users\Ann")),
                temp: PathBuf::from(r"C:\Users\Ann\AppData\Local\Temp"),
            }
        }

        #[test]
        fn user_folders_and_temp_are_cluttered_but_subfolders_are_not() {
            let known = folders();
            assert_eq!(
                classify(Path::new(r"C:\Users\Ann\OneDrive\Desktop"), &known),
                Some(Clutter::UserFolder)
            );
            assert_eq!(
                classify(Path::new(r"c:\users\ann\downloads"), &known),
                Some(Clutter::UserFolder)
            );
            assert_eq!(
                classify(Path::new(r"C:\Users\Ann\Documents\"), &known),
                Some(Clutter::UserFolder)
            );
            assert_eq!(
                classify(
                    Path::new(r"C:\Users\Ann\AppData\Local\Temp\Temp1_k0t1k.zip"),
                    &known
                ),
                Some(Clutter::Temp)
            );
            assert_eq!(
                classify(Path::new(r"C:\Users\Ann\OneDrive\Desktop\_k0t1k"), &known),
                None
            );
            assert_eq!(classify(Path::new(r"C:\Games\_k0t1k"), &known), None);
            assert_eq!(classify(Path::new(r"C:\Users\Ann"), &known), None);
            assert_eq!(
                classify(Path::new(r"C:\Users\Ann\AppData\Local\Temporary"), &known),
                None
            );
        }

        #[test]
        fn removal_refuses_roots_and_folders_of_the_user() {
            let known = folders();
            for refused in [
                r"C:\",
                r"C:\Users",
                r"C:\Users\Ann",
                r"C:\Users\Ann\Downloads",
                r"C:\Users\Ann\OneDrive\Desktop",
                r"c:\users\ann\documents",
                r"C:\Users\Ann\AppData\Local\Temp",
                r"C:\Users\Ann\AppData\Local\Temp\",
            ] {
                assert!(
                    check_removable(Path::new(refused), &known).is_err(),
                    "{refused}"
                );
            }
            for allowed in [
                r"C:\Users\Ann\AppData\Local\Programs\_k0t1k",
                r"C:\Users\Ann\OneDrive\Desktop\_k0t1k",
                r"D:\Games\_k0t1k",
            ] {
                assert!(
                    check_removable(Path::new(allowed), &known).is_ok(),
                    "{allowed}"
                );
            }
            assert!(check_removable(Path::new("relative"), &known).is_err());
        }

        #[test]
        fn same_dir_ignores_case() {
            assert!(same_dir(Path::new(r"C:\A\B"), Path::new(r"c:\a\b")));
            assert!(!same_dir(Path::new(r"C:\A\B"), Path::new(r"C:\A\C")));
        }
    }

    /// The same rules with `/` paths, which are only absolute on Unix.
    #[cfg(unix)]
    #[test]
    fn classification_and_removal_guard_with_injected_folders() {
        let known = KnownFolders {
            desktop: Some(PathBuf::from("/home/ann/OneDrive/Desktop")),
            downloads: Some(PathBuf::from("/home/ann/Downloads")),
            documents: Some(PathBuf::from("/home/ann/Documents")),
            profile: Some(PathBuf::from("/home/ann")),
            temp: PathBuf::from("/tmp-k0t1k-test"),
        };
        assert_eq!(
            classify(Path::new("/home/ann/OneDrive/Desktop"), &known),
            Some(Clutter::UserFolder)
        );
        assert_eq!(
            classify(Path::new("/HOME/ann/downloads/"), &known),
            Some(Clutter::UserFolder)
        );
        assert_eq!(
            classify(Path::new("/tmp-k0t1k-test/Temp1_k0t1k.zip"), &known),
            Some(Clutter::Temp)
        );
        assert_eq!(
            classify(Path::new("/home/ann/OneDrive/Desktop/_k0t1k"), &known),
            None
        );
        assert_eq!(classify(Path::new("/home/ann"), &known), None);
        assert_eq!(classify(Path::new("/tmp-k0t1k-testing"), &known), None);

        for refused in [
            "/",
            "/home",
            "/home/ann",
            "/home/ann/Downloads",
            "/HOME/ann/documents",
            "/tmp-k0t1k-test",
        ] {
            assert!(
                check_removable(Path::new(refused), &known).is_err(),
                "{refused}"
            );
        }
        for allowed in [
            "/home/ann/.local/share/_k0t1k",
            "/home/ann/OneDrive/Desktop/_k0t1k",
            "/opt/_k0t1k",
        ] {
            assert!(
                check_removable(Path::new(allowed), &known).is_ok(),
                "{allowed}"
            );
        }
        assert!(check_removable(Path::new("relative"), &known).is_err());
        assert!(same_dir(Path::new("/A/B"), Path::new("/a/b")));
    }

    #[test]
    fn install_target_nests_only_into_foreign_folders() {
        let root = tempfile::tempdir().unwrap();
        let empty = root.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(install_target(&empty, "_k0t1k.exe").unwrap(), empty);
        let missing = root.path().join("missing");
        assert_eq!(install_target(&missing, "_k0t1k.exe").unwrap(), missing);

        let foreign = root.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("photo.jpg"), b"x").unwrap();
        assert_eq!(
            install_target(&foreign, "_k0t1k.exe").unwrap(),
            foreign.join("_k0t1k")
        );

        // A folder that already holds a launcher is reused (reinstall / update).
        let ours = root.path().join("ours");
        std::fs::create_dir(&ours).unwrap();
        std::fs::write(ours.join("_k0t1k.exe"), b"x").unwrap();
        assert_eq!(install_target(&ours, "_k0t1k.exe").unwrap(), ours);

        std::fs::create_dir(foreign.join("_k0t1k")).unwrap();
        std::fs::write(foreign.join("_k0t1k").join("other.txt"), b"x").unwrap();
        assert!(install_target(&foreign, "_k0t1k.exe").is_err());
        assert!(install_target(Path::new("relative"), "_k0t1k.exe").is_err());
    }

    #[test]
    fn removal_plan_lists_only_owned_entries() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("_k0t1k");
        std::fs::create_dir_all(dir.join("instances")).unwrap();
        std::fs::write(dir.join("settings.json"), b"{}").unwrap();
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        let exe = dir.join("_k0t1k.exe");
        std::fs::write(&exe, b"x").unwrap();
        let lnk = root.path().join("_k0t1k.lnk");
        std::fs::write(&lnk, b"x").unwrap();
        let gone = root.path().join("gone.lnk");

        let known = KnownFolders {
            desktop: None,
            downloads: None,
            documents: None,
            profile: None,
            temp: PathBuf::from("/elsewhere/tmp"),
        };
        let plan = RemovalPlan::new(&dir, &exe, &[lnk.clone(), gone], &known).unwrap();
        assert_eq!(plan.folders, std::slice::from_ref(&dir));
        assert_eq!(
            plan.entries,
            [dir.join("instances"), dir.join("settings.json"), exe]
        );
        assert_eq!(plan.shortcuts, [lnk]);
        let script = plan.cmd_script().unwrap();
        assert!(script.starts_with("ping "));
        assert!(!script.contains("notes.txt"));
        assert_eq!(
            script.matches("rmdir \"").count(),
            2,
            "the folder itself is removed non-recursively, once per pass"
        );

        let percent = RemovalPlan {
            folders: vec![PathBuf::from(r"C:\100%\x")],
            entries: vec![],
            shortcuts: vec![],
        };
        assert!(percent.cmd_script().is_err());
    }

    #[test]
    fn owned_names_cover_the_data_layout() {
        let paths = crate::paths::Paths::new("/r");
        for path in [
            paths.meta_dir(),
            paths.libraries_dir(),
            paths.assets_dir(),
            paths.natives_dir(),
            paths.java_dir(),
            paths.instances_dir(),
            paths.shared_dir(),
            paths.cache_dir(),
            paths.settings_file(),
        ] {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            assert!(OWNED_NAMES.contains(&name.as_str()), "{name}");
        }
    }
}
