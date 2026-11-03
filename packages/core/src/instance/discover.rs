//! Finding launchers of the MultiMC family (Prism, PolyMC, MultiMC, UltimMC, Fjord) anywhere on disk by the
//! file name of their executable, including portable installs, and listing their instances. Nothing is
//! hashed or downloaded, and the walk is bounded in depth and time.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::prism::{PrismInstanceInfo, PrismLauncher, portable_data_dir, scan_data_dir, standard_data_dirs};
use crate::i18n::tr;
use crate::progress::{ProgressEvent, ProgressFn};

const TIME_BUDGET: Duration = Duration::from_secs(20);
/// Levels below a search root that are looked into (the root itself is level 0).
const ROOT_DEPTH: u32 = 4;
const HOME_DEPTH: u32 = 5;

/// Folders that are never worth walking: system, package caches, game libraries and build output.
fn is_skipped_dir(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with('.')
        || matches!(
            lower.as_str(),
            "windows"
                | "windows.old"
                | "$recycle.bin"
                | "system volume information"
                | "recovery"
                | "programdata"
                | "appdata"
                | "node_modules"
                | "target"
                | "steamapps"
                | "proc"
                | "sys"
                | "dev"
        )
}

/// Hidden, system and reparse-point (link, junction) folders are skipped; on Unix only links are.
#[cfg(windows)]
fn is_hidden_or_linked(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const REPARSE_POINT: u32 = 0x400;
    entry.metadata().map_or(true, |meta| meta.file_attributes() & (HIDDEN | SYSTEM | REPARSE_POINT) != 0)
}

#[cfg(not(windows))]
fn is_hidden_or_linked(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().map_or(true, |kind| kind.is_symlink())
}

struct Search<'a> {
    deadline: Instant,
    visited: HashSet<PathBuf>,
    found: Vec<(PathBuf, PrismLauncher)>,
    on_dir: &'a dyn Fn(),
}

impl Search<'_> {
    /// Looks for launcher executables in `dir` and in the folders below it, `depth` more levels at most.
    /// Stops descending into a folder that holds a launcher. Returns false when the time ran out.
    fn walk(&mut self, dir: &Path, depth: u32) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        // The same folder can be reached from several roots (home contains Desktop).
        if !self.visited.insert(dir.to_owned()) {
            return true;
        }
        (self.on_dir)();
        let Ok(entries) = std::fs::read_dir(dir) else { return true };
        let mut subdirs = Vec::new();
        let mut has_launcher = false;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_file() {
                if let Some(launcher) = PrismLauncher::from_exe_name(&name)
                    && let Some(data_dir) = portable_data_dir(dir, launcher)
                {
                    self.found.push((data_dir, launcher));
                    has_launcher = true;
                }
            } else if kind.is_dir() && depth > 0 && !is_skipped_dir(&name) && !is_hidden_or_linked(&entry) {
                subdirs.push(entry.path());
            }
        }
        if has_launcher {
            return true;
        }
        subdirs.into_iter().all(|subdir| self.walk(&subdir, depth - 1))
    }
}

#[cfg(windows)]
fn fixed_drives() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_FIXED: u32 = 3;
    // SAFETY: both calls only read system state; the path is a NUL-terminated UTF-16 string.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .filter(|bit| {
            let root: Vec<u16> = format!("{}:\\", (b'A' + bit) as char).encode_utf16().chain(Some(0)).collect();
            unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
        })
        .map(|bit| PathBuf::from(format!("{}:\\", (b'A' + bit) as char)))
        .collect()
}

#[cfg(not(windows))]
fn fixed_drives() -> Vec<PathBuf> {
    Vec::new()
}

/// Folders to search with how deep to go, the specific ones first: they are walked before home or a
/// whole drive, which then skips what was already seen.
fn search_roots() -> Vec<(PathBuf, u32)> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        roots.extend([dirs::desktop_dir(), dirs::download_dir(), dirs::document_dir()].into_iter().flatten().map(|dir| (dir, ROOT_DEPTH)));
        roots.extend(dirs::data_local_dir().map(|dir| (dir.join("Programs"), ROOT_DEPTH)));
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            roots.extend(std::env::var_os(variable).map(|dir| (PathBuf::from(dir), ROOT_DEPTH)));
        }
        roots.extend(dirs::home_dir().map(|dir| (dir, HOME_DEPTH)));
        roots.extend(fixed_drives().into_iter().map(|drive| (drive, ROOT_DEPTH)));
    } else {
        roots.extend(dirs::home_dir().map(|dir| (dir, HOME_DEPTH)));
        roots.extend(["/opt", "/Applications"].map(|dir| (PathBuf::from(dir), ROOT_DEPTH)));
    }
    roots
}

/// Instances of every launcher of the family on this machine: the standard data folders and the
/// portable installs found by their executable. The same instance folder is listed once. Progress counts
/// the folders looked at; the walk ends after about 20 seconds with what was found by then.
pub fn discover_launcher_instances(task: &str, progress: &ProgressFn) -> Vec<PrismInstanceInfo> {
    let stage = tr("Searching for launchers", "Поиск лаунчеров");
    let scanned = std::cell::Cell::new(0u64);
    let on_dir = || {
        scanned.set(scanned.get() + 1);
        progress(ProgressEvent { task: task.to_owned(), stage: stage.to_owned(), done: scanned.get(), total: 0 });
    };
    let mut search = Search { deadline: Instant::now() + TIME_BUDGET, visited: HashSet::new(), found: Vec::new(), on_dir: &on_dir };
    for (root, depth) in search_roots() {
        if !search.walk(&root, depth) {
            tracing::warn!("the search for launchers ran out of time");
            break;
        }
    }

    let mut instances = Vec::new();
    for (data_dir, launcher) in standard_data_dirs().into_iter().chain(search.found) {
        scan_data_dir(&data_dir, launcher, &mut instances);
    }
    let mut seen = HashSet::new();
    instances.retain(|info| seen.insert(std::fs::canonicalize(&info.path).unwrap_or_else(|_| info.path.clone())));
    instances.sort_by_key(|info| info.name.to_lowercase());
    instances
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn search(root: &Path, depth: u32) -> Vec<(PathBuf, PrismLauncher)> {
        let mut search = Search { deadline: Instant::now() + Duration::from_secs(10), visited: HashSet::new(), found: Vec::new(), on_dir: &|| {} };
        assert!(search.walk(root, depth));
        search.found
    }

    #[test]
    fn matches_executable_names_case_insensitively() {
        assert_eq!(PrismLauncher::from_exe_name("prismlauncher.exe"), Some(PrismLauncher::Prism));
        assert_eq!(PrismLauncher::from_exe_name("PrismLauncher"), Some(PrismLauncher::Prism));
        assert_eq!(PrismLauncher::from_exe_name("MultiMC.exe"), Some(PrismLauncher::MultiMC));
        assert_eq!(PrismLauncher::from_exe_name("POLYMC.EXE"), Some(PrismLauncher::PolyMC));
        assert_eq!(PrismLauncher::from_exe_name("UltimMC.exe"), Some(PrismLauncher::UltimMC));
        assert_eq!(PrismLauncher::from_exe_name("fjordlauncher"), Some(PrismLauncher::Fjord));
        assert_eq!(PrismLauncher::from_exe_name("prismlauncher_updater.exe"), None);
        assert_eq!(PrismLauncher::from_exe_name("prismlauncher.cfg"), None);
    }

    #[test]
    fn data_dir_is_the_executable_folder_only_for_portable_installs() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("installed");
        assert_eq!(portable_data_dir(&installed, PrismLauncher::Prism), None);
        std::fs::create_dir_all(&installed).unwrap();
        assert_eq!(portable_data_dir(&installed, PrismLauncher::Prism), None);
        // MultiMC and UltimMC keep everything beside the executable
        assert_eq!(portable_data_dir(&installed, PrismLauncher::MultiMC), Some(installed.clone()));
        assert_eq!(portable_data_dir(&installed, PrismLauncher::UltimMC), Some(installed.clone()));

        touch(&installed.join("portable.txt"));
        assert_eq!(portable_data_dir(&installed, PrismLauncher::Prism), Some(installed.clone()));
        let with_cfg = dir.path().join("cfg");
        touch(&with_cfg.join("polymc.cfg"));
        assert_eq!(portable_data_dir(&with_cfg, PrismLauncher::PolyMC), Some(with_cfg.clone()));
        assert_eq!(portable_data_dir(&with_cfg, PrismLauncher::Prism), None);
        let with_instances = dir.path().join("inst");
        std::fs::create_dir_all(with_instances.join("instances")).unwrap();
        assert_eq!(portable_data_dir(&with_instances, PrismLauncher::Fjord), Some(with_instances.clone()));
    }

    #[test]
    fn skips_system_hidden_and_heavy_folders() {
        for name in ["Windows", "$Recycle.Bin", "System Volume Information", "ProgramData", "AppData", "node_modules", ".git", ".cache", "target", "steamapps"] {
            assert!(is_skipped_dir(name), "{name}");
        }
        for name in ["Games", "Program Files", "Desktop", "PrismLauncher-Windows-Portable-11.1.1"] {
            assert!(!is_skipped_dir(name), "{name}");
        }
    }

    #[test]
    fn finds_portable_launchers_within_the_depth_and_outside_skipped_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("Prism-Portable/PrismLauncher.exe"));
        touch(&root.join("Prism-Portable/portable.txt"));
        touch(&root.join("Prism-Portable/instances/nested/MultiMC.exe"));
        touch(&root.join("a/b/MultiMC/MultiMC.exe"));
        touch(&root.join("installed/prismlauncher.exe"));
        touch(&root.join("node_modules/pkg/polymc.exe"));
        touch(&root.join(".hidden/polymc.exe"));
        touch(&root.join("a/b/c/d/e/UltimMC.exe"));

        let mut found = search(root, 4);
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(found, [(root.join("Prism-Portable"), PrismLauncher::Prism), (root.join("a/b/MultiMC"), PrismLauncher::MultiMC)]);
        // the deep one is within reach when the search goes deeper
        assert!(search(root, 6).contains(&(root.join("a/b/c/d/e"), PrismLauncher::UltimMC)));
    }

    #[test]
    fn does_not_descend_into_a_launcher_folder_or_follow_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("Prism/prismlauncher.exe"));
        touch(&root.join("Prism/portable.txt"));
        touch(&root.join("Prism/sub/multimc.exe"));
        touch(&root.join("elsewhere/MultiMC/MultiMC.exe"));
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("elsewhere"), root.join("link")).unwrap();

        let mut found = search(root, 4);
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(found, [(root.join("Prism"), PrismLauncher::Prism), (root.join("elsewhere/MultiMC"), PrismLauncher::MultiMC)]);
    }

    #[test]
    fn stops_when_the_time_is_up() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("MultiMC/MultiMC.exe"));
        let mut search = Search { deadline: Instant::now(), visited: HashSet::new(), found: Vec::new(), on_dir: &|| {} };
        assert!(!search.walk(dir.path(), 4));
        assert!(search.found.is_empty());
    }
}
