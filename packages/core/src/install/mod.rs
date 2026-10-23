//! Downloads everything a [`LaunchProfile`] needs and prepares natives and assets for launch.

mod assets;

use std::io;
use std::path::{Path, PathBuf};

use crate::download::{DownloadItem, download_all};
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;
use crate::profile::LaunchProfile;
use crate::progress::ProgressFn;

pub use assets::{AssetIndex, AssetObject, load_asset_index, reconstruct_assets};

/// True when `path` exists and, if `size` is known, has exactly that size. Files are written
/// atomically, so a file of the right size is trusted without hashing it again.
async fn is_present(path: &Path, size: Option<u64>) -> Result<bool> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(size.is_none_or(|size| metadata.len() == size)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).at(path),
    }
}

/// Download items for the profile's library files that are not on disk yet.
async fn missing_library_items(paths: &Paths, profile: &LaunchProfile) -> Result<Vec<DownloadItem>> {
    let libraries_dir = paths.libraries_dir();
    let mut items = Vec::new();
    for file in profile.library_files() {
        let path = libraries_dir.join(&file.path);
        if is_present(&path, file.size).await? {
            continue;
        }
        items.push(DownloadItem { url: file.url, path, sha1: file.sha1 });
    }
    Ok(items)
}

/// Downloads the main jar, libraries, natives, maven files, the asset index and all asset objects.
/// Stages reported through `progress`: `Libraries`, `Asset index`, `Assets`.
pub async fn install_game(
    client: &reqwest::Client,
    paths: &Paths,
    profile: &LaunchProfile,
    task: &str,
    progress: &ProgressFn,
) -> Result<AssetIndex> {
    let libraries = missing_library_items(paths, profile).await?;
    download_all(client, libraries, task, "Libraries", progress).await?;

    let index_ref = &profile.asset_index;
    let index_item = DownloadItem {
        url: index_ref.url.clone(),
        path: assets::index_path(paths, &index_ref.id)?,
        sha1: index_ref.sha1.clone(),
    };
    download_all(client, vec![index_item], task, "Asset index", progress).await?;

    let index = load_asset_index(paths, &index_ref.id).await?;
    let objects = assets::missing_object_items(paths, &index).await?;
    download_all(client, objects, task, "Assets", progress).await?;
    Ok(index)
}

/// Verifies that every file needed to launch is on disk, for launching without network access.
/// Returns the asset index so the caller does not have to load it again.
pub async fn check_installed(paths: &Paths, profile: &LaunchProfile) -> Result<AssetIndex> {
    let mut missing: Vec<PathBuf> =
        missing_library_items(paths, profile).await?.into_iter().map(|item| item.path).collect();

    let index_path = assets::index_path(paths, &profile.asset_index.id)?;
    if !is_present(&index_path, None).await? {
        missing.push(index_path);
        return Err(missing_error(&missing));
    }
    let index = load_asset_index(paths, &profile.asset_index.id).await?;
    missing.extend(assets::missing_object_items(paths, &index).await?.into_iter().map(|item| item.path));

    if missing.is_empty() { Ok(index) } else { Err(missing_error(&missing)) }
}

fn missing_error(missing: &[PathBuf]) -> Error {
    Error::Other(format!("{} game files are missing, first: {}", missing.len(), missing[0].display()))
}

/// Unzips every native jar of the profile into `natives_dir`, overwriting existing files.
/// The caller owns the directory, including removing stale files from other versions.
pub async fn extract_natives(paths: &Paths, profile: &LaunchProfile, natives_dir: &Path) -> Result<()> {
    let libraries_dir = paths.libraries_dir();
    let jars: Vec<PathBuf> = profile.native_files().iter().map(|file| libraries_dir.join(&file.path)).collect();
    let natives_dir = natives_dir.to_owned();
    let rename_jnilib = profile.runtime.system == "osx";

    tokio::task::spawn_blocking(move || extract_jars(&jars, &natives_dir, rename_jnilib))
        .await
        .map_err(|error| Error::Other(format!("natives extraction task failed: {error}")))?
}

fn extract_jars(jars: &[PathBuf], natives_dir: &Path, rename_jnilib: bool) -> Result<()> {
    std::fs::create_dir_all(natives_dir).at(natives_dir)?;
    for jar in jars {
        let file = std::fs::File::open(jar).at(jar)?;
        let mut archive = zip::ZipArchive::new(file)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .enclosed_name()
                .ok_or_else(|| Error::Other(format!("{} contains an entry with an unsafe path", jar.display())))?;
            let mut target = natives_dir.join(name);
            if rename_jnilib && target.extension().is_some_and(|extension| extension == "jnilib") {
                target.set_extension("dylib");
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            let mut output = std::fs::File::create(&target).at(&target)?;
            io::copy(&mut entry, &mut output).at(&target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn extracts_native_jars_and_renames_jnilib() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("natives.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("liblwjgl.so", options).unwrap();
        zip.write_all(b"so").unwrap();
        zip.start_file("sub/libopenal.jnilib", options).unwrap();
        zip.write_all(b"jni").unwrap();
        zip.finish().unwrap();

        let out = dir.path().join("out");
        extract_jars(std::slice::from_ref(&jar), &out, true).unwrap();
        assert_eq!(std::fs::read(out.join("liblwjgl.so")).unwrap(), b"so");
        assert_eq!(std::fs::read(out.join("sub/libopenal.dylib")).unwrap(), b"jni");
        assert!(!out.join("sub/libopenal.jnilib").exists());
    }

    #[test]
    fn rejects_zip_slip_entries() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("evil.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();

        let out = dir.path().join("out");
        assert!(extract_jars(std::slice::from_ref(&jar), &out, false).is_err());
        assert!(!dir.path().join("escape.txt").exists());
    }
}
