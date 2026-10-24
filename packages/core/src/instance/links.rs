//! Sharing folders (`resourcepacks`, `shaderpacks`, `screenshots`) between instances through one
//! folder per kind, `shared/<folder>`. The server list (`servers.dat`) is a file, so it is shared as a
//! synchronised copy instead (`servers::pull_shared`).
//!
//! The game dir entry becomes a symlink on unix and a directory junction on Windows. A junction
//! needs no administrator rights or Developer Mode, unlike a symlink, and works for a local directory.
//!
//! Future named pools (different shared sets for groups of instances) would add a pool id to the
//! shared path, `shared/<pool>/<folder>`, next to the `shared_dir().join(folder.dir_name())` below;
//! `SharedFolder` and the stored `sharedFolders` list (the implicit default pool) need no migration.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::fsutil::{copy_dir, is_link, remove_link, unique_path};
use super::servers;
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

/// A game folder that can be shared between instances. Mods, config and saves are never shareable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SharedFolder {
    Resourcepacks,
    Shaderpacks,
    Screenshots,
    /// The server list, `servers.dat`. A file the game replaces on save, so it is synchronised
    /// instead of linked (see `servers::pull_shared`).
    Servers,
}

impl SharedFolder {
    pub const ALL: [SharedFolder; 4] = [Self::Resourcepacks, Self::Shaderpacks, Self::Screenshots, Self::Servers];

    /// Name of the entry in the game dir and under `shared/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Resourcepacks => "resourcepacks",
            Self::Shaderpacks => "shaderpacks",
            Self::Screenshots => "screenshots",
            Self::Servers => "servers.dat",
        }
    }

    /// Shared through a directory link; `false` for the synchronised file.
    pub(super) fn is_linked(self) -> bool {
        self != Self::Servers
    }
}

/// True when `path` already exists inside a linked shared folder of `game_dir`, i.e. it is a
/// file that a modpack must not replace.
pub fn exists_in_linked_shared_folder(game_dir: &Path, path: &Path) -> bool {
    path.exists()
        && SharedFolder::ALL.iter().filter(|folder| folder.is_linked()).any(|folder| {
            let link = game_dir.join(folder.dir_name());
            path.starts_with(&link) && is_link(&link)
        })
}

/// Make `<game_dir>/<folder>` point at the shared folder. Content already in a real folder is
/// moved into the shared one first (renamed on a name clash), so no user file is deleted.
/// Calling it again is harmless.
pub fn enable_shared(paths: &Paths, game_dir: &Path, folder: SharedFolder) -> Result<()> {
    if !folder.is_linked() {
        return servers::enable_shared_list(paths, game_dir);
    }
    let shared = paths.shared_dir().join(folder.dir_name());
    std::fs::create_dir_all(&shared).at(&shared)?;
    let link = game_dir.join(folder.dir_name());

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

/// Replace the link with a real folder holding a copy of the shared content, so the instance
/// keeps working with the same files. Does nothing when the folder is not a link.
pub fn disable_shared(paths: &Paths, game_dir: &Path, folder: SharedFolder) -> Result<()> {
    // The instance keeps its copy of the server list.
    if !folder.is_linked() {
        return Ok(());
    }
    let link = game_dir.join(folder.dir_name());
    if !is_link(&link) {
        return Ok(());
    }
    remove_link(&link)?;
    let shared = paths.shared_dir().join(folder.dir_name());
    if shared.is_dir() {
        copy_dir(&shared, &link, &|_| false)
    } else {
        std::fs::create_dir_all(&link).at(&link)
    }
}

/// Remove every shared-folder link of a game dir, leaving the shared content untouched. Used before
/// deleting an instance so a recursive delete can never reach shared files.
pub fn detach_all(game_dir: &Path) -> Result<()> {
    for folder in SharedFolder::ALL.into_iter().filter(|folder| folder.is_linked()) {
        let link = game_dir.join(folder.dir_name());
        if is_link(&link) {
            remove_link(&link)?;
        }
    }
    Ok(())
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
    use std::path::PathBuf;

    use super::*;

    const PACKS: SharedFolder = SharedFolder::Resourcepacks;

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
        enable_shared(&paths, &game, PACKS).unwrap();
        enable_shared(&paths, &game, PACKS).unwrap();

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

        enable_shared(&paths, &game, PACKS).unwrap();

        assert_eq!(std::fs::read(shared.join("same.zip")).unwrap(), b"shared");
        assert_eq!(std::fs::read(shared.join("same (1).zip")).unwrap(), b"mine");
        assert!(shared.join("folder-pack/pack.mcmeta").is_file());
    }

    #[test]
    fn disable_keeps_a_private_copy_and_leaves_shared_intact() {
        let (_dir, paths, game) = setup();
        enable_shared(&paths, &game, PACKS).unwrap();
        std::fs::write(game.join("resourcepacks/p.zip"), "pack").unwrap();

        disable_shared(&paths, &game, PACKS).unwrap();
        let folder = game.join("resourcepacks");
        assert!(!is_link(&folder) && folder.is_dir());
        assert_eq!(std::fs::read(folder.join("p.zip")).unwrap(), b"pack");
        assert!(paths.shared_dir().join("resourcepacks/p.zip").is_file());

        // Not a link any more: a second disable is a no-op.
        disable_shared(&paths, &game, PACKS).unwrap();
    }

    #[test]
    fn regular_file_in_the_way_is_an_error() {
        let (_dir, paths, game) = setup();
        std::fs::write(game.join("resourcepacks"), "oops").unwrap();
        assert!(enable_shared(&paths, &game, PACKS).is_err());
    }

    #[test]
    fn each_folder_is_shared_independently_with_clash_renaming() {
        let (_dir, paths, game) = setup();
        for folder in [SharedFolder::Shaderpacks, SharedFolder::Screenshots] {
            let shared = paths.shared_dir().join(folder.dir_name());
            std::fs::create_dir_all(&shared).unwrap();
            std::fs::write(shared.join("a.png"), "shared").unwrap();
            std::fs::create_dir_all(game.join(folder.dir_name())).unwrap();
            std::fs::write(game.join(folder.dir_name()).join("a.png"), "mine").unwrap();

            enable_shared(&paths, &game, folder).unwrap();
            assert!(is_link(&game.join(folder.dir_name())));
            assert_eq!(std::fs::read(shared.join("a.png")).unwrap(), b"shared");
            assert_eq!(std::fs::read(shared.join("a (1).png")).unwrap(), b"mine");
        }
        assert!(!is_link(&game.join("resourcepacks")));

        disable_shared(&paths, &game, SharedFolder::Screenshots).unwrap();
        assert!(!is_link(&game.join("screenshots")));
        assert!(game.join("screenshots/a (1).png").is_file());
        assert!(is_link(&game.join("shaderpacks")));
    }

    #[test]
    fn detach_all_removes_links_but_keeps_shared_content() {
        let (_dir, paths, game) = setup();
        for folder in SharedFolder::ALL.into_iter().filter(|folder| folder.is_linked()) {
            enable_shared(&paths, &game, folder).unwrap();
            std::fs::write(paths.shared_dir().join(folder.dir_name()).join("f"), "x").unwrap();
        }
        detach_all(&game).unwrap();
        for folder in SharedFolder::ALL.into_iter().filter(|folder| folder.is_linked()) {
            assert!(!game.join(folder.dir_name()).exists());
            assert!(paths.shared_dir().join(folder.dir_name()).join("f").is_file());
        }
    }

    #[test]
    fn only_existing_files_in_linked_folders_are_protected() {
        let (_dir, paths, game) = setup();
        enable_shared(&paths, &game, SharedFolder::Shaderpacks).unwrap();
        std::fs::write(game.join("shaderpacks/s.zip"), "x").unwrap();
        std::fs::create_dir_all(game.join("resourcepacks")).unwrap();
        std::fs::write(game.join("resourcepacks/r.zip"), "x").unwrap();

        assert!(exists_in_linked_shared_folder(&game, &game.join("shaderpacks/s.zip")));
        assert!(!exists_in_linked_shared_folder(&game, &game.join("shaderpacks/new.zip")));
        assert!(!exists_in_linked_shared_folder(&game, &game.join("resourcepacks/r.zip")));
    }
}
