//! Sharing `resourcepacks` between instances through one folder, `shared/resourcepacks`.
//!
//! The game dir entry becomes a symlink on unix and a directory junction on Windows. A junction
//! needs no administrator rights or Developer Mode, unlike a symlink, and works for a local directory.

use std::path::{Path, PathBuf};

use super::fsutil::{copy_dir, is_link, remove_link, unique_path};
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

fn shared_resourcepacks(paths: &Paths) -> PathBuf {
    paths.shared_dir().join("resourcepacks")
}

/// Make `<game_dir>/resourcepacks` point at the shared folder. Packs already in a real folder are
/// moved into the shared one first (renamed on a name clash), so no user file is deleted.
/// Calling it again is harmless.
pub fn enable_shared_resourcepacks(paths: &Paths, game_dir: &Path) -> Result<()> {
    let shared = shared_resourcepacks(paths);
    std::fs::create_dir_all(&shared).at(&shared)?;
    let link = game_dir.join("resourcepacks");

    if is_link(&link) {
        remove_link(&link)?;
    } else if link.is_dir() {
        for entry in std::fs::read_dir(&link).at(&link)? {
            let entry = entry.at(&link)?;
            let target = unique_path(&shared, &entry.file_name().to_string_lossy());
            std::fs::rename(entry.path(), &target).at(&target)?;
        }
        std::fs::remove_dir(&link).at(&link)?;
    } else if link.exists() {
        return Err(Error::Other(format!(
            "{} exists and is not a directory",
            link.display()
        )));
    }

    std::fs::create_dir_all(game_dir).at(game_dir)?;
    create_dir_link(&shared, &link)
}

/// Replace the link with a real folder holding a copy of the shared packs, so the instance
/// keeps working with the same packs. Does nothing when `resourcepacks` is not a link.
pub fn disable_shared_resourcepacks(paths: &Paths, game_dir: &Path) -> Result<()> {
    let link = game_dir.join("resourcepacks");
    if !is_link(&link) {
        return Ok(());
    }
    remove_link(&link)?;
    let shared = shared_resourcepacks(paths);
    if shared.is_dir() {
        copy_dir(&shared, &link, &|_| false)
    } else {
        std::fs::create_dir_all(&link).at(&link)
    }
}

#[cfg(unix)]
fn create_dir_link(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).at(link)
}

#[cfg(windows)]
fn create_dir_link(target: &Path, link: &Path) -> Result<()> {
    junction::create(target, link).at(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Paths, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"));
        let game = dir.path().join("data/instances/a/minecraft");
        std::fs::create_dir_all(&game).unwrap();
        (dir, paths, game)
    }

    #[test]
    fn enable_links_to_shared_folder() {
        let (_dir, paths, game) = setup();
        enable_shared_resourcepacks(&paths, &game).unwrap();
        enable_shared_resourcepacks(&paths, &game).unwrap();

        let link = game.join("resourcepacks");
        assert!(is_link(&link));
        std::fs::write(paths.shared_dir().join("resourcepacks/p.zip"), "pack").unwrap();
        assert_eq!(std::fs::read(link.join("p.zip")).unwrap(), b"pack");
    }

    #[test]
    fn existing_packs_are_moved_not_lost_or_overwritten() {
        let (_dir, paths, game) = setup();
        let shared = paths.shared_dir().join("resourcepacks");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("same.zip"), "shared").unwrap();
        std::fs::create_dir_all(game.join("resourcepacks/folder-pack")).unwrap();
        std::fs::write(game.join("resourcepacks/same.zip"), "mine").unwrap();
        std::fs::write(game.join("resourcepacks/folder-pack/pack.mcmeta"), "{}").unwrap();

        enable_shared_resourcepacks(&paths, &game).unwrap();

        assert_eq!(std::fs::read(shared.join("same.zip")).unwrap(), b"shared");
        assert_eq!(std::fs::read(shared.join("same (1).zip")).unwrap(), b"mine");
        assert!(shared.join("folder-pack/pack.mcmeta").is_file());
    }

    #[test]
    fn disable_keeps_a_private_copy_and_leaves_shared_intact() {
        let (_dir, paths, game) = setup();
        enable_shared_resourcepacks(&paths, &game).unwrap();
        std::fs::write(game.join("resourcepacks/p.zip"), "pack").unwrap();

        disable_shared_resourcepacks(&paths, &game).unwrap();
        let folder = game.join("resourcepacks");
        assert!(!is_link(&folder) && folder.is_dir());
        assert_eq!(std::fs::read(folder.join("p.zip")).unwrap(), b"pack");
        assert!(paths.shared_dir().join("resourcepacks/p.zip").is_file());

        // Not a link any more: a second disable is a no-op.
        disable_shared_resourcepacks(&paths, &game).unwrap();
    }

    #[test]
    fn regular_file_in_the_way_is_an_error() {
        let (_dir, paths, game) = setup();
        std::fs::write(game.join("resourcepacks"), "oops").unwrap();
        assert!(enable_shared_resourcepacks(&paths, &game).is_err());
    }
}
