//! Managed Java runtimes downloaded from Mojang's runtime index into `java/<component>`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::detect::executables_in;
use crate::download::{DownloadItem, download_all, get_json};
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;
use crate::progress::ProgressFn;

const RUNTIME_INDEX_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// `{ platform: { component: [entry] } }`; a component unavailable on a platform has an empty list.
type RuntimeIndex = HashMap<String, HashMap<String, Vec<RuntimeEntry>>>;

#[derive(Deserialize)]
struct RuntimeEntry {
    manifest: Download,
}

#[derive(Deserialize)]
struct Download {
    sha1: String,
    url: String,
}

#[derive(Deserialize)]
struct Manifest {
    files: BTreeMap<String, ManifestFile>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ManifestFile {
    Directory,
    File {
        // Only read on unix; Windows has no executable bit.
        #[cfg_attr(not(unix), allow(dead_code))]
        #[serde(default)]
        executable: bool,
        downloads: FileDownloads,
    },
    Link {
        // Only read on unix, where links are created.
        #[cfg_attr(not(unix), allow(dead_code))]
        target: String,
    },
}

#[derive(Deserialize)]
struct FileDownloads {
    raw: Download,
}

/// Key of the running OS and CPU in Mojang's runtime index.
pub fn mojang_platform() -> Result<&'static str> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "x86") => "windows-x86",
        ("windows", "aarch64") => "windows-arm64",
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        ("macos", "x86_64") => "mac-os",
        ("macos", "aarch64") => "mac-os-arm64",
        (os, arch) => return Err(Error::Other(format!("Mojang provides no Java runtime for {os}/{arch}"))),
    };
    Ok(platform)
}

/// Mojang runtime component that ships the given Java major.
pub fn component_for_major(major: u32) -> Option<&'static str> {
    match major {
        8 => Some("jre-legacy"),
        16 => Some("java-runtime-alpha"),
        17 => Some("java-runtime-gamma"),
        21 => Some("java-runtime-delta"),
        25 => Some("java-runtime-epsilon"),
        _ => None,
    }
}

/// Game executable of an installed managed runtime, `None` when it is not (completely) installed.
pub fn managed_java_path(paths: &Paths, component: &str) -> Option<PathBuf> {
    executables_in(&paths.java_dir().join(component)).into_iter().next()
}

/// Install `component` for the current platform unless it is already installed: an installed
/// runtime may be in use by a running game, so it is never replaced.
/// Files are downloaded into `java/.tmp/<id>` and moved into place only when everything is complete,
/// so an interrupted install never looks valid. Returns the game executable.
pub async fn install_runtime(
    paths: &Paths,
    client: &reqwest::Client,
    component: &str,
    progress: &ProgressFn,
) -> Result<PathBuf> {
    if let Some(installed) = managed_java_path(paths, component) {
        return Ok(installed);
    }
    let platform = mojang_platform()?;
    let mut index: RuntimeIndex = get_json(client, RUNTIME_INDEX_URL).await?;
    let entry = index
        .remove(platform)
        .and_then(|mut components| components.remove(component))
        .and_then(|entries| entries.into_iter().next())
        .ok_or_else(|| Error::Other(format!("Java runtime {component} is not available for {platform}")))?;
    let manifest: Manifest = get_json(client, &entry.manifest.url).await?;
    verify_manifest_paths(&manifest)?;

    let java_dir = paths.java_dir();
    let staging = java_dir.join(".tmp").join(uuid::Uuid::new_v4().simple().to_string());
    let task = format!("java:{component}");
    let result = stage_runtime(client, &manifest, &staging, &task, progress).await;
    if let Err(error) = result {
        remove_staging(&staging);
        return Err(error);
    }

    let destination = java_dir.join(component);
    let moved = (|| {
        // Only an incomplete copy can be here: a complete one returned early above.
        if destination.exists() {
            std::fs::remove_dir_all(&destination).at(&destination)?;
        }
        std::fs::rename(&staging, &destination).at(&destination)
    })();
    if let Err(error) = moved {
        remove_staging(&staging);
        return Err(error);
    }

    remove_empty_dir(&java_dir.join(".tmp"));
    managed_java_path(paths, component).ok_or_else(|| {
        Error::Other(format!(
            "Java runtime {component} was installed but contains no java executable"
        ))
    })
}

async fn stage_runtime(
    client: &reqwest::Client,
    manifest: &Manifest,
    staging: &Path,
    task: &str,
    progress: &ProgressFn,
) -> Result<()> {
    let mut items = Vec::new();
    for (relative, file) in &manifest.files {
        let target = staging.join(relative);
        match file {
            ManifestFile::Directory => std::fs::create_dir_all(&target).at(&target)?,
            ManifestFile::File { downloads, .. } => items.push(DownloadItem {
                url: downloads.raw.url.clone(),
                path: target,
                sha1: Some(downloads.raw.sha1.clone()),
            }),
            ManifestFile::Link { .. } => {}
        }
    }
    std::fs::create_dir_all(staging).at(staging)?;

    download_all(client, items, task, "Downloading Java runtime", progress).await?;

    #[cfg(unix)]
    apply_unix_metadata(manifest, staging)?;
    Ok(())
}

/// Executable bits and symlinks. Windows skips both: it has no executable bit, and the links
/// only point at duplicate license files under `legal/`, as in Prism.
#[cfg(unix)]
fn apply_unix_metadata(manifest: &Manifest, staging: &Path) -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};

    for (relative, file) in &manifest.files {
        let target = staging.join(relative);
        match file {
            ManifestFile::File { executable: true, .. } => {
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).at(&target)?;
            }
            ManifestFile::Link { target: link_target } => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).at(parent)?;
                }
                symlink(link_target, &target).at(&target)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Reject manifest entries that would be written outside the runtime directory.
fn verify_manifest_paths(manifest: &Manifest) -> Result<()> {
    for relative in manifest.files.keys() {
        let escapes = Path::new(relative)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)));
        if escapes {
            return Err(Error::Other(format!(
                "Java runtime manifest contains an unsafe path: {relative}"
            )));
        }
    }
    Ok(())
}

fn remove_staging(staging: &Path) {
    if let Err(error) = std::fs::remove_dir_all(staging)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove Java staging dir {}: {error}", staging.display());
    }
    if let Some(root) = staging.parent() {
        remove_empty_dir(root);
    }
}

/// `remove_dir` only succeeds on an empty folder, so the staging dir of another install is never touched.
fn remove_empty_dir(dir: &Path) {
    let _ = std::fs::remove_dir(dir);
}

/// Startup cleanup: installs are never resumed, so whatever `java/.tmp` holds is an interrupted one.
pub fn remove_leftover_staging(paths: &Paths) {
    let staging = paths.java_dir().join(".tmp");
    if let Err(error) = std::fs::remove_dir_all(&staging)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove leftover Java staging dir {}: {error}", staging.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_and_manifest_shapes() {
        let index: RuntimeIndex = serde_json::from_str(
            r#"{"linux":{"java-runtime-delta":[{"availability":{"group":1,"progress":100},
                "manifest":{"sha1":"ab","size":1,"url":"https://x/m.json"},
                "version":{"name":"21.0.7","released":"2025"}}],"jre-legacy":[]}}"#,
        )
        .unwrap();
        assert_eq!(index["linux"]["java-runtime-delta"][0].manifest.url, "https://x/m.json");
        assert!(index["linux"]["jre-legacy"].is_empty());

        let manifest: Manifest = serde_json::from_str(
            r#"{"files":{"bin":{"type":"directory"},
                "bin/java":{"type":"file","executable":true,"downloads":{"raw":{"sha1":"cd","size":2,"url":"https://x/j"},
                    "lzma":{"sha1":"ef","size":1,"url":"https://x/jl"}}},
                "legal/a":{"type":"link","target":"../b/a"}}}"#,
        )
        .unwrap();
        assert_eq!(manifest.files.len(), 3);
        assert!(matches!(manifest.files["bin/java"], ManifestFile::File { executable: true, .. }));
        verify_manifest_paths(&manifest).unwrap();
    }

    #[test]
    fn unsafe_manifest_paths_are_rejected() {
        for bad in ["../evil", "/abs", "a/../../b"] {
            let manifest: Manifest =
                serde_json::from_str(&format!(r#"{{"files":{{"{bad}":{{"type":"directory"}}}}}}"#)).unwrap();
            assert!(verify_manifest_paths(&manifest).is_err(), "{bad}");
        }
    }

    #[test]
    fn major_to_component() {
        assert_eq!(component_for_major(8), Some("jre-legacy"));
        assert_eq!(component_for_major(21), Some("java-runtime-delta"));
        assert_eq!(component_for_major(11), None);
    }
}
