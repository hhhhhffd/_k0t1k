//! Worlds of an instance (`<game dir>/saves/<world>`): info from `level.dat`, rename, copy, delete,
//! zip backup and import. Blocking file I/O; call from `spawn_blocking`. These functions do not know
//! whether the game is running; callers must refuse to modify a world while it is.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::fsutil::{copy_dir, unique_path};
use super::nbt::{self, Tag};
use crate::error::{Error, IoContext, Result};

const SAVES_FOLDER: &str = "saves";
const LEVEL_FILE: &str = "level.dat";
const ICON_FILE: &str = "icon.png";
const DATAPACKS_FOLDER: &str = "datapacks";
/// Held open by the game while the world is loaded; it is never part of a copy or a backup.
const LOCK_FILE: &str = "session.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
    /// Folder name under `saves/`.
    pub dir_name: String,
    /// `LevelName` of `level.dat`; the folder name when the file cannot be read.
    pub name: String,
    pub game_mode: Option<GameMode>,
    pub hardcore: Option<bool>,
    /// Minecraft version that last saved the world (absent in worlds older than 1.9).
    pub version_name: Option<String>,
    pub last_played: Option<DateTime<Utc>>,
    pub seed: Option<i64>,
    #[serde(skip)]
    pub icon_png: Option<Vec<u8>>,
    /// Bytes on disk of the whole world folder.
    pub size: u64,
}

fn saves_dir(game_dir: &Path) -> PathBuf {
    game_dir.join(SAVES_FOLDER)
}

/// The folder of an existing world; `dir_name` must be a plain folder name.
pub fn world_dir(game_dir: &Path, dir_name: &str) -> Result<PathBuf> {
    let mut components = Path::new(dir_name).components();
    let path = match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => saves_dir(game_dir).join(dir_name),
        _ => return Err(Error::Other(format!("{dir_name:?} is not a plain folder name"))),
    };
    if !path.is_dir() {
        return Err(Error::Other(format!("world {dir_name:?} does not exist")));
    }
    Ok(path)
}

/// Turns a display name into a folder name Windows accepts. Fails when nothing usable is left.
fn folder_name(name: &str) -> Result<String> {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
        "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let clean: String = name.chars().filter(|c| !c.is_control() && !"\\/:*?\"<>|".contains(*c)).collect();
    let clean = clean.trim().trim_end_matches('.').trim_end().to_owned();
    let stem = clean.split('.').next().unwrap_or_default();
    if clean.is_empty() || RESERVED.iter().any(|reserved| stem.eq_ignore_ascii_case(reserved)) {
        return Err(Error::Other(format!("{name:?} cannot be used as a world folder name")));
    }
    Ok(clean)
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut size = 0;
    for entry in std::fs::read_dir(path).at(path)? {
        let entry = entry.at(path)?;
        let file_type = entry.file_type().at(entry.path())?;
        size += if file_type.is_dir() {
            dir_size(&entry.path())?
        } else {
            entry.metadata().at(entry.path())?.len()
        };
    }
    Ok(size)
}

fn read_level(world_dir: &Path) -> Result<(String, nbt::Compound)> {
    nbt::read_file(&world_dir.join(LEVEL_FILE))
}

/// Reads one world folder. A `level.dat` that cannot be parsed still yields a world, with the folder
/// name and no details, so the user can rename or delete a broken world.
fn read_world(path: &Path, dir_name: String) -> Result<World> {
    let level = path.join(LEVEL_FILE);
    let modified: Option<DateTime<Utc>> = std::fs::metadata(&level).and_then(|meta| meta.modified()).ok().map(Into::into);
    let mut world = World {
        name: dir_name.clone(),
        dir_name,
        game_mode: None,
        hardcore: None,
        version_name: None,
        last_played: modified,
        seed: None,
        icon_png: std::fs::read(path.join(ICON_FILE)).ok(),
        size: dir_size(path)?,
    };
    let data = match read_level(path) {
        Ok((_, root)) => root.compound("Data").cloned(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", level.display());
            None
        }
    };
    let Some(data) = data else { return Ok(world) };
    if let Some(name) = data.string("LevelName") {
        world.name = name.to_owned();
    }
    world.game_mode = data.integer("GameType").and_then(|mode| match mode {
        0 => Some(GameMode::Survival),
        1 => Some(GameMode::Creative),
        2 => Some(GameMode::Adventure),
        3 => Some(GameMode::Spectator),
        _ => None,
    });
    world.hardcore = data.integer("hardcore").map(|flag| flag != 0);
    world.version_name = data.compound("Version").and_then(|version| version.string("Name")).map(str::to_owned);
    if let Some(played) = data.integer("LastPlayed").and_then(DateTime::from_timestamp_millis) {
        world.last_played = Some(played);
    }
    // Before 1.16 the seed is `RandomSeed`; since then it is `WorldGenSettings.seed`.
    world.seed = data
        .compound("WorldGenSettings")
        .and_then(|settings| settings.integer("seed"))
        .or_else(|| data.integer("RandomSeed"));
    Ok(world)
}

/// Worlds that have a `level.dat`, most recently played first.
pub fn list(game_dir: &Path) -> Result<Vec<World>> {
    let saves = saves_dir(game_dir);
    let entries = match std::fs::read_dir(&saves) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(&saves),
    };
    let mut worlds = Vec::new();
    for entry in entries {
        let entry = entry.at(&saves)?;
        let path = entry.path();
        if path.is_dir() && path.join(LEVEL_FILE).is_file() {
            worlds.push(read_world(&path, entry.file_name().to_string_lossy().into_owned())?);
        }
    }
    worlds.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.dir_name.cmp(&b.dir_name)));
    Ok(worlds)
}

fn set_level_name(world_dir: &Path, name: &str) -> Result<()> {
    let (root_name, mut root) = read_level(world_dir)?;
    let data = root
        .compound_mut("Data")
        .ok_or_else(|| Error::Other(format!("{} has no Data tag", world_dir.join(LEVEL_FILE).display())))?;
    data.set("LevelName", Tag::String(name.to_owned()));
    nbt::write_file(&world_dir.join(LEVEL_FILE), &root_name, &root, true)
}

/// Sets the display name (`LevelName`) and, when `rename_folder` is set, moves the folder to a name
/// derived from it. Returns the folder name the world has afterwards.
pub fn rename(game_dir: &Path, dir_name: &str, new_name: &str, rename_folder: bool) -> Result<String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(Error::Other("the world name is empty".into()));
    }
    let path = world_dir(game_dir, dir_name)?;
    let target = if rename_folder {
        let target = saves_dir(game_dir).join(folder_name(new_name)?);
        // A case-only change names the same folder on Windows, which is fine.
        if target != path && target.exists() && !target.as_os_str().eq_ignore_ascii_case(path.as_os_str()) {
            return Err(Error::Other(format!("a world folder {:?} already exists", target.file_name().unwrap_or_default())));
        }
        Some(target)
    } else {
        None
    };
    set_level_name(&path, new_name)?;
    let Some(target) = target else { return Ok(dir_name.to_owned()) };
    if target != path {
        std::fs::rename(&path, &target).at(&target)?;
    }
    Ok(target.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

/// Copies the world into a new folder named after `new_name` (made unique when taken) and gives the
/// copy that display name. Returns the new folder name.
pub fn copy(game_dir: &Path, dir_name: &str, new_name: &str) -> Result<String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(Error::Other("the world name is empty".into()));
    }
    let source = world_dir(game_dir, dir_name)?;
    let target = unique_path(&saves_dir(game_dir), &folder_name(new_name)?);
    copy_dir(&source, &target, &|relative| relative == Path::new(LOCK_FILE))?;
    if let Err(error) = set_level_name(&target, new_name) {
        let _ = std::fs::remove_dir_all(&target);
        return Err(error);
    }
    Ok(target.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

/// Deletes the world folder for good.
pub fn delete(game_dir: &Path, dir_name: &str) -> Result<()> {
    let path = world_dir(game_dir, dir_name)?;
    std::fs::remove_dir_all(&path).at(&path)
}

fn add_to_zip(
    writer: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    prefix: &str,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).at(dir)? {
        let entry = entry.at(dir)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let entry_name = format!("{prefix}/{name}");
        let file_type = entry.file_type().at(&path)?;
        if file_type.is_dir() {
            writer.add_directory(&entry_name, options)?;
            add_to_zip(writer, &path, &entry_name, options)?;
        } else if file_type.is_file() && name != LOCK_FILE {
            writer.start_file(&entry_name, options)?;
            let mut file = std::fs::File::open(&path).at(&path)?;
            std::io::copy(&mut file, writer).at(&path)?;
        }
    }
    Ok(())
}

/// Zips the world folder (as the single top-level folder of the archive) into the file `destination`.
/// A failed backup leaves no partial file behind.
pub fn backup(game_dir: &Path, dir_name: &str, destination: &Path) -> Result<()> {
    let source = world_dir(game_dir, dir_name)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    let file = std::fs::File::create(destination).at(destination)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let result = writer
        .add_directory(format!("{dir_name}/"), options)
        .map_err(Error::from)
        .and_then(|()| add_to_zip(&mut writer, &source, dir_name, options))
        .and_then(|()| writer.finish().map(drop).map_err(Error::from));
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

/// A path from a zip entry as relative components. Entries that could land outside the extraction
/// folder (`..`, absolute paths, drive letters) make the whole archive untrusted.
fn safe_relative(entry_name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::new();
    for part in entry_name.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(Error::Other(format!("unsafe path in the archive: {entry_name:?}"))),
            part if part.contains(':') => return Err(Error::Other(format!("unsafe path in the archive: {entry_name:?}"))),
            part => path.push(part),
        }
    }
    Ok(path)
}

/// Extracts a world archive into `saves/`. The world is the shallowest folder holding a `level.dat`
/// (archives often wrap it in an extra folder or are the world's contents themselves). Returns the
/// folder name it was given.
pub fn import_zip(game_dir: &Path, archive_path: &Path) -> Result<String> {
    let file = std::fs::File::open(archive_path).at(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        let is_symlink = entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000);
        entries.push((safe_relative(&entry.name()?)?, entry.is_dir(), is_symlink));
    }
    let root = entries
        .iter()
        .filter(|(path, is_dir, _)| !is_dir && path.file_name().is_some_and(|name| name == LEVEL_FILE))
        .filter(|(path, ..)| path.components().next().is_none_or(|first| first.as_os_str() != "__MACOSX"))
        .map(|(path, ..)| path.parent().unwrap_or(Path::new("")).to_path_buf())
        .min_by_key(|parent| parent.components().count())
        .ok_or_else(|| Error::Other(format!("{} contains no world (level.dat not found)", archive_path.display())))?;

    let display_name = match root.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => archive_path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
    };
    let saves = saves_dir(game_dir);
    std::fs::create_dir_all(&saves).at(&saves)?;
    let target = unique_path(&saves, &folder_name(&display_name)?);

    let extracted = extract_world(&mut archive, &entries, &root, &target);
    if extracted.is_err() {
        let _ = std::fs::remove_dir_all(&target);
    }
    extracted?;
    Ok(target.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

fn extract_world(
    archive: &mut zip::ZipArchive<std::fs::File>,
    entries: &[(PathBuf, bool, bool)],
    root: &Path,
    target: &Path,
) -> Result<()> {
    std::fs::create_dir_all(target).at(target)?;
    for (index, (path, is_dir, is_symlink)) in entries.iter().enumerate() {
        let Ok(relative) = path.strip_prefix(root) else { continue };
        if relative.as_os_str().is_empty() || *is_symlink {
            continue;
        }
        let destination = target.join(relative);
        if *is_dir {
            std::fs::create_dir_all(&destination).at(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let mut output = std::fs::File::create(&destination).at(&destination)?;
        let mut input = archive.by_index(index)?;
        std::io::copy(&mut input, &mut output).at(&destination)?;
        output.flush().at(&destination)?;
    }
    Ok(())
}

/// Copies a `.zip` data pack into the world's `datapacks/` folder (renamed when the name is taken)
/// and returns the file name it has there.
pub fn add_datapack(game_dir: &Path, dir_name: &str, source: &Path) -> Result<String> {
    let world = world_dir(game_dir, dir_name)?;
    let file_name = source
        .file_name()
        .filter(|_| source.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip")))
        .ok_or_else(|| Error::Other(format!("{} is not a .zip data pack", source.display())))?
        .to_string_lossy()
        .into_owned();
    // A data pack that is not a zip with a pack.mcmeta would be silently ignored by the game.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(source).at(source)?)?;
    let mut mcmeta = String::new();
    match archive.by_name("pack.mcmeta") {
        Ok(mut entry) => {
            entry.read_to_string(&mut mcmeta).at(source)?;
        }
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(Error::Other(format!("{} is not a data pack (no pack.mcmeta)", source.display())));
        }
        Err(error) => return Err(error.into()),
    }
    let folder = world.join(DATAPACKS_FOLDER);
    std::fs::create_dir_all(&folder).at(&folder)?;
    let target = unique_path(&folder, &file_name);
    std::fs::copy(source, &target).at(&target)?;
    Ok(target.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbt::Compound;

    fn level(name: &str, seed: i64) -> Compound {
        Compound(vec![(
            "Data".into(),
            Tag::Compound(Compound(vec![
                ("LevelName".into(), Tag::String(name.into())),
                ("GameType".into(), Tag::Int(1)),
                ("hardcore".into(), Tag::Byte(1)),
                ("LastPlayed".into(), Tag::Long(1_700_000_000_000)),
                ("Version".into(), Tag::Compound(Compound(vec![("Name".into(), Tag::String("1.21.1".into()))]))),
                (
                    "WorldGenSettings".into(),
                    Tag::Compound(Compound(vec![("seed".into(), Tag::Long(seed))])),
                ),
                ("DataVersion".into(), Tag::Int(3955)),
            ])),
        )])
    }

    fn make_world(game_dir: &Path, dir_name: &str, name: &str) -> PathBuf {
        let path = game_dir.join("saves").join(dir_name);
        std::fs::create_dir_all(path.join("region")).unwrap();
        nbt::write_file(&path.join("level.dat"), "", &level(name, -42), true).unwrap();
        std::fs::write(path.join("region/r.0.0.mca"), [1, 2, 3]).unwrap();
        std::fs::write(path.join("session.lock"), "lock").unwrap();
        std::fs::write(path.join("icon.png"), [0x89, b'P']).unwrap();
        path
    }

    #[test]
    fn list_reads_level_dat() {
        let dir = tempfile::tempdir().unwrap();
        make_world(dir.path(), "w1", "Мой мир");
        std::fs::create_dir_all(dir.path().join("saves/not-a-world")).unwrap();
        let broken = dir.path().join("saves/broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("level.dat"), "garbage").unwrap();

        let worlds = list(dir.path()).unwrap();
        assert_eq!(worlds.len(), 2);
        let world = worlds.iter().find(|world| world.dir_name == "w1").unwrap();
        assert_eq!(world.name, "Мой мир");
        assert_eq!(world.game_mode, Some(GameMode::Creative));
        assert_eq!(world.hardcore, Some(true));
        assert_eq!(world.version_name.as_deref(), Some("1.21.1"));
        assert_eq!(world.seed, Some(-42));
        assert_eq!(world.last_played, DateTime::from_timestamp_millis(1_700_000_000_000));
        assert_eq!(world.icon_png.as_deref(), Some(&[0x89, b'P'][..]));
        assert!(world.size >= 3);
        let broken = worlds.iter().find(|world| world.dir_name == "broken").unwrap();
        assert_eq!((broken.name.as_str(), broken.seed), ("broken", None));
        assert!(list(&dir.path().join("none")).unwrap().is_empty());
    }

    #[test]
    fn old_worlds_use_random_seed() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_world(dir.path(), "old", "Old");
        let (_, mut root) = nbt::read_file(&path.join("level.dat")).unwrap();
        let data = root.compound_mut("Data").unwrap();
        data.0.retain(|(key, _)| key != "WorldGenSettings");
        data.set("RandomSeed", Tag::Long(77));
        nbt::write_file(&path.join("level.dat"), "", &root, true).unwrap();
        assert_eq!(list(dir.path()).unwrap()[0].seed, Some(77));
    }

    #[test]
    fn rename_keeps_unknown_tags_and_can_move_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        make_world(dir.path(), "w1", "Old");
        make_world(dir.path(), "taken", "Other");

        assert_eq!(rename(dir.path(), "w1", "New name", false).unwrap(), "w1");
        let (_, root) = nbt::read_file(&dir.path().join("saves/w1/level.dat")).unwrap();
        let data = root.compound("Data").unwrap();
        assert_eq!(data.string("LevelName"), Some("New name"));
        assert_eq!(data.integer("DataVersion"), Some(3955));

        assert!(rename(dir.path(), "w1", "taken", true).is_err());
        assert_eq!(rename(dir.path(), "w1", "New: name?", true).unwrap(), "New name");
        assert!(dir.path().join("saves/New name/level.dat").is_file());
        assert!(!dir.path().join("saves/w1").exists());
        assert!(rename(dir.path(), "New name", "  ", false).is_err());
        assert!(rename(dir.path(), "../x", "a", false).is_err());
    }

    #[test]
    fn copy_skips_the_session_lock_and_renames_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        make_world(dir.path(), "w1", "Old");
        assert_eq!(copy(dir.path(), "w1", "Copy").unwrap(), "Copy");
        assert_eq!(copy(dir.path(), "w1", "Copy").unwrap(), "Copy (1)");
        assert!(dir.path().join("saves/Copy/region/r.0.0.mca").is_file());
        assert!(!dir.path().join("saves/Copy/session.lock").exists());
        let worlds = list(dir.path()).unwrap();
        assert!(worlds.iter().any(|world| world.dir_name == "Copy" && world.name == "Copy"));
        assert!(worlds.iter().any(|world| world.dir_name == "w1" && world.name == "Old"));

        delete(dir.path(), "Copy").unwrap();
        assert!(!dir.path().join("saves/Copy").exists());
        assert!(delete(dir.path(), "missing").is_err());
    }

    #[test]
    fn backup_then_import_restores_the_world() {
        let dir = tempfile::tempdir().unwrap();
        make_world(dir.path(), "w1", "Backed up");
        let zip_path = dir.path().join("backups/w1.zip");
        backup(dir.path(), "w1", &zip_path).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned()).collect();
        assert!(names.contains(&"w1/level.dat".to_owned()));
        assert!(names.contains(&"w1/region/r.0.0.mca".to_owned()));
        assert!(!names.iter().any(|name| name.ends_with("session.lock")));

        let other = tempfile::tempdir().unwrap();
        assert_eq!(import_zip(other.path(), &zip_path).unwrap(), "w1");
        assert_eq!(import_zip(other.path(), &zip_path).unwrap(), "w1 (1)");
        assert_eq!(list(other.path()).unwrap()[0].name, "Backed up");
        assert_eq!(std::fs::read(other.path().join("saves/w1/region/r.0.0.mca")).unwrap(), [1, 2, 3]);
    }

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, content) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn import_rejects_zip_slip_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        for evil in ["../evil.txt", "world/../../evil.txt", "..\\evil.txt", "C:/evil.txt"] {
            let zip_path = dir.path().join("evil.zip");
            zip_with(&zip_path, &[("world/level.dat", b"x"), (evil, b"pwn")]);
            let error = import_zip(dir.path(), &zip_path).unwrap_err().to_string();
            assert!(error.contains("unsafe path"), "{evil}: {error}");
        }
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!dir.path().join("saves").exists());
    }

    #[test]
    fn import_finds_the_shallowest_level_dat() {
        let dir = tempfile::tempdir().unwrap();
        let wrapped = dir.path().join("wrapped.zip");
        zip_with(
            &wrapped,
            &[
                ("pack/readme.txt", b"hi"),
                ("pack/My World/level.dat", b"x"),
                ("pack/My World/data/a.dat", b"a"),
                ("pack/My World/dim/level.dat", b"nested"),
                ("__MACOSX/level.dat", b"junk"),
            ],
        );
        assert_eq!(import_zip(dir.path(), &wrapped).unwrap(), "My World");
        assert!(dir.path().join("saves/My World/data/a.dat").is_file());
        assert!(!dir.path().join("saves/My World/readme.txt").exists());

        let flat = dir.path().join("Flat Save.zip");
        zip_with(&flat, &[("level.dat", b"x"), ("region/r.mca", b"r")]);
        assert_eq!(import_zip(dir.path(), &flat).unwrap(), "Flat Save");
        assert!(dir.path().join("saves/Flat Save/region/r.mca").is_file());

        let none = dir.path().join("none.zip");
        zip_with(&none, &[("a.txt", b"x")]);
        assert!(import_zip(dir.path(), &none).unwrap_err().to_string().contains("no world"));
    }

    #[test]
    fn folder_names_are_cleaned() {
        assert_eq!(folder_name(" My: World. ").unwrap(), "My World");
        assert!(folder_name("???").is_err());
        assert!(folder_name("con").is_err());
        assert!(folder_name("Nul.txt").is_err());
    }

    #[test]
    fn add_datapack_requires_a_zip_with_pack_mcmeta() {
        let dir = tempfile::tempdir().unwrap();
        make_world(dir.path(), "w1", "W");
        let good = dir.path().join("good.zip");
        zip_with(&good, &[("pack.mcmeta", b"{}")]);
        assert_eq!(add_datapack(dir.path(), "w1", &good).unwrap(), "good.zip");
        assert_eq!(add_datapack(dir.path(), "w1", &good).unwrap(), "good (1).zip");

        let bad = dir.path().join("bad.zip");
        zip_with(&bad, &[("readme.txt", b"x")]);
        assert!(add_datapack(dir.path(), "w1", &bad).unwrap_err().to_string().contains("pack.mcmeta"));
        let text = dir.path().join("a.txt");
        std::fs::write(&text, "x").unwrap();
        assert!(add_datapack(dir.path(), "w1", &text).is_err());
    }
}
