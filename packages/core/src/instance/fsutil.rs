use std::path::{Path, PathBuf};

use crate::error::{IoContext, Result};

/// True for symlinks and, on Windows, directory junctions.
pub fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Remove a link itself, never its target.
pub fn remove_link(path: &Path) -> Result<()> {
    // A directory symlink is removed with `remove_file` on unix but needs `remove_dir` on Windows.
    #[cfg(unix)]
    let removed = std::fs::remove_file(path);
    #[cfg(windows)]
    let removed = std::fs::remove_dir(path);
    removed.at(path)
}

/// Recursively copy `src` into `dst`, following links. `skip` gets each entry's path relative to `src`.
pub fn copy_dir(src: &Path, dst: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<()> {
    copy_dir_inner(src, dst, Path::new(""), skip)
}

fn copy_dir_inner(src: &Path, dst: &Path, relative: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<()> {
    std::fs::create_dir_all(dst).at(dst)?;
    for entry in std::fs::read_dir(src).at(src)? {
        let entry = entry.at(src)?;
        let entry_relative = relative.join(entry.file_name());
        if skip(&entry_relative) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if std::fs::metadata(&from).at(&from)?.is_dir() {
            copy_dir_inner(&from, &to, &entry_relative, skip)?;
        } else {
            std::fs::copy(&from, &to).at(&from)?;
        }
    }
    Ok(())
}

/// `dir/name`, or `dir/name (n)` keeping the extension, whichever does not exist yet.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .expect("an unbounded range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_keeps_extension() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.zip"), dir.path().join("a.zip"));
        std::fs::write(dir.path().join("a.zip"), "").unwrap();
        assert_eq!(unique_path(dir.path(), "a.zip"), dir.path().join("a (1).zip"));
        std::fs::write(dir.path().join("a (1).zip"), "").unwrap();
        assert_eq!(unique_path(dir.path(), "a.zip"), dir.path().join("a (2).zip"));
    }

    #[test]
    fn copy_dir_applies_skip_to_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("keep/deep")).unwrap();
        std::fs::create_dir_all(src.join("skip")).unwrap();
        std::fs::write(src.join("keep/deep/f.txt"), "x").unwrap();
        std::fs::write(src.join("skip/f.txt"), "y").unwrap();

        let dst = dir.path().join("dst");
        copy_dir(&src, &dst, &|rel| rel == Path::new("skip")).unwrap();
        assert!(dst.join("keep/deep/f.txt").is_file());
        assert!(!dst.join("skip").exists());
    }
}
