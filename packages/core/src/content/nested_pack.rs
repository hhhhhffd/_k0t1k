//! Packs distributed with one extra folder: `pack.zip/Folder/pack.mcmeta` or `resourcepacks/X/Y/pack.mcmeta`.
//! Minecraft only looks for `pack.mcmeta` at the root, so such a pack is invisible to it.

use std::io::Write;
use std::path::Path;

use crate::error::{Error, IoContext, Result};

const PACK_MCMETA: &str = "pack.mcmeta";
const PACK_MCMETA_DISABLED: &str = "pack.mcmeta.disabled";
const MAX_DEPTH: usize = 32;

fn is_junk(path: &str) -> bool {
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    path.starts_with("__MACOSX/") || matches!(name, "__MACOSX" | ".DS_Store" | "Thumbs.db" | "desktop.ini")
}

/// The folder chain (`"A/"`, `"A/B/"`) that holds all entries of a zip and a `pack.mcmeta`, when the
/// archive has no root `pack.mcmeta`. Junk entries (`__MACOSX/`, `.DS_Store`...) are ignored.
pub fn zip_nested_root(names: &[String]) -> Option<String> {
    let mut common: Option<Vec<&str>> = None;
    let entries = || names.iter().filter(|name| !is_junk(name));
    for name in entries().filter(|name| !name.ends_with('/')) {
        let mut parts: Vec<&str> = name.split('/').collect();
        parts.pop();
        common = Some(match common {
            None => parts,
            Some(previous) => previous.into_iter().zip(parts).take_while(|(a, b)| a == b).map(|(a, _)| a).collect(),
        });
    }
    let prefix = format!("{}/", common.filter(|parts| !parts.is_empty())?.join("/"));
    // A folder entry may be an ancestor of the root, but not lie beside it.
    let folders_fit = entries().filter(|name| name.ends_with('/')).all(|name| prefix.starts_with(name.as_str()) || name.starts_with(&prefix));
    (folders_fit && names.iter().any(|name| *name == format!("{prefix}{PACK_MCMETA}"))).then_some(prefix)
}

fn zip_names(path: &Path) -> Result<Vec<String>> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path).at(path)?)?;
    let mut names = Vec::with_capacity(archive.len());
    for name in archive.file_names() {
        names.push(name?.into_owned());
    }
    Ok(names)
}

fn has_mcmeta(folder: &Path) -> bool {
    folder.join(PACK_MCMETA).is_file() || folder.join(PACK_MCMETA_DISABLED).is_file()
}

/// Relative path (`"Y/"`, `"Y/Z/"`) of the single-folder chain leading to the real root of a folder
/// pack that has no `pack.mcmeta` of its own.
pub fn folder_nested_root(folder: &Path) -> Option<String> {
    if has_mcmeta(folder) {
        return None;
    }
    let (mut current, mut relative) = (folder.to_path_buf(), String::new());
    for _ in 0..MAX_DEPTH {
        let mut children = std::fs::read_dir(&current)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| !is_junk(&entry.file_name().to_string_lossy()));
        let child = children.next()?;
        if children.next().is_some() || !child.file_type().ok()?.is_dir() {
            return None;
        }
        relative.push_str(&format!("{}/", child.file_name().to_string_lossy()));
        current = child.path();
        if has_mcmeta(&current) {
            return Some(relative);
        }
    }
    None
}

/// The nested root of a pack on disk (a folder or a zip); `None` when it is laid out correctly or unreadable.
pub fn nested_root_of(path: &Path) -> Option<String> {
    if path.is_dir() {
        folder_nested_root(path)
    } else {
        zip_nested_root(&zip_names(path).ok()?)
    }
}

/// Rewrites a zip so the nested folder becomes its root. The new archive is built next to the
/// original and replaces it only when complete, so a failure leaves the original untouched.
/// Returns whether the zip needed fixing.
pub fn fix_zip(path: &Path) -> Result<bool> {
    let Some(prefix) = zip_nested_root(&zip_names(path)?) else {
        return Ok(false);
    };
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{file_name}.fixing"));
    let result = write_unwrapped(path, &temp, &prefix).and_then(|()| std::fs::rename(&temp, path).at(path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map(|()| true)
}

/// Entries are copied still compressed, so their data is byte-identical.
fn write_unwrapped(source: &Path, temp: &Path, prefix: &str) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(source).at(source)?)?;
    let mut writer = zip::ZipWriter::new(std::fs::File::create_new(temp).at(temp)?);
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        let name = entry.name()?.into_owned();
        match name.strip_prefix(prefix) {
            Some(rest) if !rest.is_empty() && !is_junk(&name) => writer.raw_copy_file_rename(entry, rest)?,
            _ => {}
        }
    }
    let mut file = writer.finish()?;
    file.flush().at(temp)?;
    file.sync_all().at(temp)
}

/// Moves the content of the nested folder up into the pack folder (junk is dropped). The top folder
/// is first renamed to a hidden staging name, so an inner entry named like it cannot clash. Returns
/// whether the pack needed fixing.
pub fn fix_folder(folder: &Path) -> Result<bool> {
    let Some(relative) = folder_nested_root(folder) else {
        return Ok(false);
    };
    let (top, rest) = relative.split_once('/').unwrap_or_default();
    let staging = folder.join(".k0t1k-unwrap");
    if staging.symlink_metadata().is_ok() {
        return Err(Error::Other(format!("{} already exists", staging.display())));
    }
    let inner = staging.join(rest);
    std::fs::rename(folder.join(top), &staging).at(&staging)?;

    let mut moved = Vec::new();
    let result = move_children(&inner, folder, &mut moved);
    if result.is_err() {
        for name in moved {
            let _ = std::fs::rename(folder.join(&name), inner.join(&name));
        }
        let _ = std::fs::rename(&staging, folder.join(top));
        return result.map(|()| false);
    }
    // Only junk and now-empty folders are left in the staging folder.
    std::fs::remove_dir_all(&staging).at(&staging)?;
    Ok(true)
}

fn move_children(inner: &Path, folder: &Path, moved: &mut Vec<std::ffi::OsString>) -> Result<()> {
    for entry in std::fs::read_dir(inner).at(inner)? {
        let name = entry.at(inner)?.file_name();
        if is_junk(&name.to_string_lossy()) {
            continue;
        }
        let target = folder.join(&name);
        // `rename` would silently replace a file on some platforms.
        if target.symlink_metadata().is_ok() {
            return Err(Error::Other(format!("{} already exists", target.display())));
        }
        std::fs::rename(inner.join(&name), &target).at(&target)?;
        moved.push(name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    fn read_all(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            entries.push((entry.name().unwrap().into_owned(), bytes));
        }
        entries.sort();
        entries
    }

    #[test]
    fn detects_single_and_multi_level_roots_ignoring_junk() {
        assert_eq!(zip_nested_root(&names(&["Pack/pack.mcmeta", "Pack/assets/a.png"])), Some("Pack/".into()));
        assert_eq!(
            zip_nested_root(&names(&["__MACOSX/Pack/._x", ".DS_Store", "Pack/", "Pack/pack.mcmeta", "Pack/Thumbs.db"])),
            Some("Pack/".into())
        );
        assert_eq!(zip_nested_root(&names(&["a/b/pack.mcmeta", "a/b/assets/x.png", "a/"])), Some("a/b/".into()));
    }

    #[test]
    fn correct_or_unrelated_layouts_are_not_nested() {
        assert_eq!(zip_nested_root(&names(&["pack.mcmeta", "assets/a.png"])), None);
        assert_eq!(zip_nested_root(&names(&["pack.mcmeta", "Pack/pack.mcmeta"])), None);
        assert_eq!(zip_nested_root(&names(&["A/pack.mcmeta", "B/x.png"])), None);
        assert_eq!(zip_nested_root(&names(&["Pack/assets/a.png"])), None);
        assert_eq!(zip_nested_root(&names(&["Pack/pack.mcmeta", "readme.txt"])), None);
    }

    #[test]
    fn rewrite_keeps_content_and_drops_junk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.zip");
        write_zip(
            &path,
            &[
                ("Pack/", b""),
                ("Pack/pack.mcmeta", b"{}"),
                ("Pack/assets/a.png", &[7u8; 5000]),
                ("Pack/.DS_Store", b"junk"),
                ("__MACOSX/Pack/._a", b"junk"),
            ],
        );
        assert!(fix_zip(&path).unwrap());
        assert_eq!(
            read_all(&path),
            [("assets/a.png".to_owned(), vec![7u8; 5000]), ("pack.mcmeta".to_owned(), b"{}".to_vec())]
        );
        assert!(!fix_zip(&path).unwrap());
        assert!(!dir.path().join(".p.zip.fixing").exists());
    }

    /// Info-ZIP "ux" (0x7875) is written by `zip` on Linux/macOS; copying such entries must not fail.
    #[test]
    fn zip_with_unix_uid_gid_extra_field_is_detected_fixed_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let mut options = zip::write::FullFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        // version 1, uid size 4 = 1000, gid size 4 = 1000
        let ux = [1u8, 4, 0xE8, 3, 0, 0, 4, 0xE8, 3, 0, 0];
        options.add_extra_data(0x7875, &ux, false).unwrap();
        for (name, bytes) in [("Pack/pack.mcmeta", b"{}".as_slice()), ("Pack/assets/a.png", b"png")] {
            writer.start_file(name, options.clone()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();

        assert_eq!(nested_root_of(&path), Some("Pack/".into()));
        assert!(fix_zip(&path).unwrap());
        assert_eq!(nested_root_of(&path), None);
        assert_eq!(read_all(&path), [("assets/a.png".to_owned(), b"png".to_vec()), ("pack.mcmeta".to_owned(), b"{}".to_vec())]);
    }

    #[test]
    fn failed_rewrite_leaves_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.zip");
        write_zip(&path, &[("Pack/pack.mcmeta", b"{}"), ("Pack/a.txt", b"a")]);
        let before = std::fs::read(&path).unwrap();
        // A leftover temp file makes the rewrite fail after detection.
        std::fs::write(dir.path().join(".p.zip.fixing"), b"x").unwrap();
        assert!(fix_zip(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        std::fs::write(dir.path().join("broken.zip"), b"not a zip").unwrap();
        assert!(fix_zip(&dir.path().join("broken.zip")).is_err());
        assert_eq!(std::fs::read(dir.path().join("broken.zip")).unwrap(), b"not a zip");
    }

    #[test]
    fn folder_pack_is_unwrapped() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("X");
        std::fs::create_dir_all(pack.join("Y/Y/assets")).unwrap();
        std::fs::write(pack.join("Y/Y/pack.mcmeta"), b"{}").unwrap();
        std::fs::write(pack.join("Y/Y/assets/a.png"), b"p").unwrap();
        std::fs::write(pack.join("Y/.DS_Store"), b"junk").unwrap();
        assert_eq!(nested_root_of(&pack), Some("Y/Y/".into()));

        assert!(fix_folder(&pack).unwrap());
        assert!(pack.join("pack.mcmeta").is_file() && pack.join("assets/a.png").is_file());
        assert!(!pack.join("Y").exists() && !pack.join(".k0t1k-unwrap").exists());
        assert_eq!(folder_nested_root(&pack), None);
        assert!(!fix_folder(&pack).unwrap());
    }

    #[test]
    fn folder_with_siblings_or_own_mcmeta_is_not_nested() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("X");
        std::fs::create_dir_all(pack.join("Y")).unwrap();
        std::fs::write(pack.join("Y/pack.mcmeta"), b"{}").unwrap();
        std::fs::write(pack.join("readme.txt"), b"r").unwrap();
        assert_eq!(folder_nested_root(&pack), None);
        std::fs::remove_file(pack.join("readme.txt")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), b"{}").unwrap();
        assert_eq!(folder_nested_root(&pack), None);
    }
}
