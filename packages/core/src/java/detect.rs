use std::collections::HashSet;
use std::path::{Path, PathBuf};

use futures::{StreamExt, stream};

use super::{JavaInstall, game_executable, probe};
use crate::paths::Paths;

const CONCURRENT_PROBES: usize = 8;
/// `runtime/<component>/<platform>/<component>/bin` is the deepest known layout.
const BUNDLE_SEARCH_DEPTH: usize = 5;

/// Find installed Javas on this machine plus our managed runtimes, newest first.
/// Candidates that fail to run are logged and left out.
pub async fn detect_java(paths: &Paths) -> Vec<JavaInstall> {
    let candidates = dedupe_by_canonical_path(candidate_executables(paths));

    let mut installs: Vec<JavaInstall> = stream::iter(candidates)
        .map(|candidate| async move {
            match probe(&candidate).await {
                Ok(install) => Some(install),
                Err(error) => {
                    tracing::debug!("skipping Java candidate: {error}");
                    None
                }
            }
        })
        .buffer_unordered(CONCURRENT_PROBES)
        .filter_map(|install| async move { install })
        .collect()
        .await;

    // Probing finishes in arbitrary order, so ties on version/arch are broken by path.
    installs.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then_with(|| b.is_64bit.cmp(&a.is_64bit))
            .then_with(|| a.path.cmp(&b.path))
    });
    installs
}

/// Executables inside a managed runtime directory (`java/<component>`), if installed.
pub(super) fn executables_in(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    find_bin_executables(dir, BUNDLE_SEARCH_DEPTH, &mut found);
    found
}

fn candidate_executables(paths: &Paths) -> Vec<PathBuf> {
    let exe = game_executable();
    let mut candidates = Vec::new();

    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(PathBuf::from(home).join("bin").join(exe));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(exe)));
    }

    // Managed runtimes: java/<component>/...
    if let Ok(entries) = std::fs::read_dir(paths.java_dir()) {
        for entry in entries.flatten() {
            // Dot-directories hold staging data of unfinished installs.
            if !entry.file_name().to_string_lossy().starts_with('.') {
                candidates.extend(executables_in(&entry.path()));
            }
        }
    }

    for root in platform_roots() {
        find_bin_executables(&root, BUNDLE_SEARCH_DEPTH, &mut candidates);
    }
    #[cfg(windows)]
    candidates.extend(registry_java_homes().into_iter().map(|home| home.join("bin").join(exe)));

    candidates
}

#[cfg(windows)]
fn platform_roots() -> Vec<PathBuf> {
    const VENDOR_DIRS: [&str; 9] = [
        "Java",
        "Eclipse Adoptium",
        "Eclipse Foundation",
        "AdoptOpenJDK",
        "Microsoft",
        "Semeru",
        "Zulu",
        "BellSoft",
        "Amazon Corretto",
    ];
    let mut roots = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(base) = std::env::var_os(var) {
            roots.extend(VENDOR_DIRS.iter().map(|vendor| Path::new(&base).join(vendor)));
        }
    }
    // Official launcher bundles: %APPDATA%\.minecraft\runtime and the Microsoft Store launcher's copy.
    if let Some(appdata) = dirs::data_dir() {
        roots.push(appdata.join(".minecraft").join("runtime"));
    }
    if let Some(local) = dirs::data_local_dir() {
        roots.push(
            local
                .join("Packages")
                .join("Microsoft.4297127D64EC6_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("Local")
                .join("runtime"),
        );
    }
    roots
}

#[cfg(not(windows))]
fn platform_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".minecraft").join("runtime"));
    }
    roots
}

/// Collect `<dir>/bin/<exe>` for `dir` under `root` (including `root` itself) without descending into a hit.
/// macOS bundles keep the runtime in `Contents/Home`, which the depth-limited walk reaches too.
fn find_bin_executables(root: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let exe = root.join("bin").join(game_executable());
    if exe.is_file() {
        found.push(exe);
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            find_bin_executables(&entry.path(), depth - 1, found);
        }
    }
}

fn dedupe_by_canonical_path(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            // Non-existent candidates fail to canonicalize and are dropped here.
            std::fs::canonicalize(candidate).is_ok_and(|canonical| seen.insert(canonical))
        })
        .collect()
}

/// `(key under SOFTWARE, value holding the home dir, subkey suffix, only the 64-bit view)`
#[cfg(windows)]
const REGISTRY_ENTRIES: [(&str, &str, &str, bool); 14] = [
    (r"JavaSoft\Java Runtime Environment", "JavaHome", "", false),
    (r"JavaSoft\Java Development Kit", "JavaHome", "", false),
    (r"JavaSoft\JRE", "JavaHome", "", false),
    (r"JavaSoft\JDK", "JavaHome", "", false),
    (r"AdoptOpenJDK\JRE", "Path", r"\hotspot\MSI", false),
    (r"AdoptOpenJDK\JDK", "Path", r"\hotspot\MSI", false),
    (r"Eclipse Foundation\JDK", "Path", r"\hotspot\MSI", false),
    (r"Eclipse Adoptium\JRE", "Path", r"\hotspot\MSI", false),
    (r"Eclipse Adoptium\JDK", "Path", r"\hotspot\MSI", false),
    (r"Semeru\JRE", "Path", r"\openj9\MSI", false),
    (r"Semeru\JDK", "Path", r"\openj9\MSI", false),
    (r"Microsoft\JDK", "Path", r"\hotspot\MSI", true),
    (r"Azul Systems\Zulu", "InstallationPath", "", false),
    (r"BellSoft\Liberica", "InstallationPath", "", false),
];

#[cfg(windows)]
fn registry_java_homes() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

    let mut homes = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for (view, is_64bit_view) in [(KEY_WOW64_64KEY, true), (KEY_WOW64_32KEY, false)] {
            let root_key = RegKey::predef(root);
            for (key, value, suffix, only_64bit) in REGISTRY_ENTRIES {
                if only_64bit && !is_64bit_view {
                    continue;
                }
                // A vendor that is not installed has no key; that is the normal case.
                let Ok(base) = root_key.open_subkey_with_flags(format!(r"SOFTWARE\{key}"), KEY_READ | view) else {
                    continue;
                };
                for subkey in base.enum_keys().flatten() {
                    let home = base
                        .open_subkey_with_flags(format!("{subkey}{suffix}"), KEY_READ | view)
                        .and_then(|k| k.get_value::<String, _>(value));
                    match home {
                        Ok(home) => homes.push(PathBuf::from(home)),
                        Err(error) => tracing::debug!("registry Java entry {key}\\{subkey} unreadable: {error}"),
                    }
                }
            }
        }
    }
    homes
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::java::test_support::write_fake_java;

    #[test]
    fn finds_nested_bundles_without_descending_into_hits() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("rt/gamma/linux/gamma/bin/java");
        let b = dir.path().join("rt/jdk-21/bin/java");
        write_fake_java(&a, "17.0.15");
        write_fake_java(&b, "21.0.7");
        write_fake_java(&dir.path().join("rt/jdk-21/bin/nested/bin/java"), "9");

        let mut found = executables_in(&dir.path().join("rt"));
        found.sort();
        assert_eq!(found, vec![a, b]);
    }

    #[test]
    fn dedupes_symlinks_and_drops_missing() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real/bin/java");
        write_fake_java(&real, "17.0.15");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("real"), &link).unwrap();

        let unique = dedupe_by_canonical_path(vec![
            real.clone(),
            link.join("bin/java"),
            dir.path().join("missing/java"),
        ]);
        assert_eq!(unique, vec![real]);
    }

    #[tokio::test]
    async fn detects_managed_runtimes_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        write_fake_java(&paths.java_dir().join("jre-legacy/bin/java"), "1.8.0_51");
        write_fake_java(&paths.java_dir().join("java-runtime-delta/bin/java"), "21.0.7");
        write_fake_java(&paths.java_dir().join(".tmp/half/bin/java"), "99.0.0");

        let installs = detect_java(&paths).await;
        let managed: Vec<u32> = installs
            .iter()
            .filter(|i| i.path.starts_with(paths.java_dir()))
            .map(|i| i.major)
            .collect();
        assert_eq!(managed, vec![21, 8]);
    }
}
