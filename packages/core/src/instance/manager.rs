use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::fsutil::{copy_dir, is_link};
use super::icon::custom_file_name as custom_icon_file;
use super::links::{SharedFolder, detach_all, disable_shared, enable_shared};
use super::{INSTANCE_FILE, Instance, InstanceSettings, ManagedPack, Playtime};
use crate::component::ComponentSpec;
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;
use crate::settings::{GlobalSettings, write_json_atomic};

const MAX_DIR_NAME_CHARS: usize = 80;
const MAX_UNIQUE_SUFFIX: u32 = 9000;
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone)]
pub struct NewInstance {
    pub name: String,
    pub icon: Option<String>,
    pub group: Option<String>,
    pub components: Vec<ComponentSpec>,
    pub managed_pack: Option<ManagedPack>,
}

/// What to bring over when copying an instance. Everything is copied by default.
#[derive(Debug, Clone)]
pub struct CopyOptions {
    pub copy_saves: bool,
    pub keep_playtime: bool,
    pub copy_game_options: bool,
    pub copy_resource_packs: bool,
    pub copy_shader_packs: bool,
    pub copy_servers: bool,
    pub copy_mods: bool,
    pub copy_screenshots: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            copy_saves: true,
            keep_playtime: true,
            copy_game_options: true,
            copy_resource_packs: true,
            copy_shader_packs: true,
            copy_servers: true,
            copy_mods: true,
            copy_screenshots: true,
        }
    }
}

impl CopyOptions {
    /// Top-level game dir entries (lowercase) that must not be copied.
    fn excluded_entries(&self) -> Vec<&'static str> {
        let groups: [(bool, &[&str]); 7] = [
            (self.copy_saves, &["saves"]),
            (self.copy_game_options, &["options.txt"]),
            (self.copy_resource_packs, &["resourcepacks", "texturepacks"]),
            (self.copy_shader_packs, &["shaderpacks"]),
            (self.copy_servers, &["servers.dat", "servers.dat_old", "server-resource-packs"]),
            (self.copy_mods, &["coremods", "mods", "config"]),
            (self.copy_screenshots, &["screenshots"]),
        ];
        groups
            .into_iter()
            .filter(|(copied, _)| !copied)
            .flat_map(|(_, names)| names.iter().copied())
            .collect()
    }
}

/// An instance directory that could not be loaded.
#[derive(Debug, Clone, Serialize)]
pub struct BrokenInstance {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InstanceList {
    pub instances: Vec<Instance>,
    pub errors: Vec<BrokenInstance>,
}

/// Operations on `instances/`. Calls are blocking file-system work; from async code run them
/// in `spawn_blocking`. Mutations are serialised by an internal lock, so share one manager.
pub struct InstanceManager {
    paths: Paths,
    lock: Mutex<()>,
}

impl InstanceManager {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            lock: Mutex::new(()),
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// All instances sorted by name; directories that fail to load are reported in `errors`.
    pub fn list(&self) -> Result<InstanceList> {
        let root = self.paths.instances_dir();
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(InstanceList::default()),
            Err(source) => return Err(Error::Io { path: root, source }),
        };

        let mut list = InstanceList::default();
        for entry in entries {
            let entry = entry.at(&root)?;
            let id = entry.file_name().to_string_lossy().into_owned();
            // Dot-directories are staging areas; directories without instance.json are not instances.
            if id.starts_with('.') || !entry.path().join(INSTANCE_FILE).is_file() {
                continue;
            }
            match self.get(&id) {
                Ok(instance) => list.instances.push(instance),
                Err(error) => list.errors.push(BrokenInstance {
                    id,
                    error: error.to_string(),
                }),
            }
        }
        list.instances.sort_by_key(|i| i.name.to_lowercase());
        list.errors.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(list)
    }

    pub fn get(&self, id: &str) -> Result<Instance> {
        let file = self.instance_dir(id)?.join(INSTANCE_FILE);
        let bytes = std::fs::read(&file).at(&file)?;
        let mut instance = Instance::from_json(&bytes)?;
        instance.id = id.to_owned();
        Ok(instance)
    }

    /// Creates an instance sharing the folders chosen in the global settings (`sharedFoldersDefault`).
    pub fn create(&self, new: NewInstance) -> Result<Instance> {
        let _guard = self.guard();
        let shared_folders = GlobalSettings::load(&self.paths)?.shared_folders_default;
        self.stage(|staging| {
            let instance = Instance {
                id: String::new(),
                name: new.name,
                icon: new.icon.unwrap_or_else(|| "default".to_owned()),
                group: normalize_group(new.group),
                components: new.components,
                created: Utc::now(),
                last_played: None,
                playtime: Playtime::default(),
                managed_pack: new.managed_pack,
                notes: String::new(),
                settings: InstanceSettings::default(),
                shared_folders,
                join_server: None,
            };
            let game_dir = staging.join("minecraft");
            std::fs::create_dir_all(&game_dir).at(&game_dir)?;
            for folder in &instance.shared_folders {
                enable_shared(&self.paths, &game_dir, *folder)?;
            }
            Ok(instance)
        })
    }

    /// Load, change and save an instance's `instance.json` (settings, notes, icon, components...).
    /// The id never changes.
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Instance)) -> Result<Instance> {
        let _guard = self.guard();
        let mut instance = self.get(id)?;
        let previous_icon = instance.icon.clone();
        change(&mut instance);
        instance.id = id.to_owned();
        self.save(&instance)?;
        // A custom icon file nothing refers to any more is garbage.
        if instance.icon != previous_icon
            && let Some(file_name) = custom_icon_file(&previous_icon)
        {
            let path = self.instance_dir(id)?.join(file_name);
            if let Err(error) = std::fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("could not remove the old instance icon {}: {error}", path.display());
            }
        }
        Ok(instance)
    }

    /// Rename in metadata only; the directory (and so the id) stays.
    pub fn rename(&self, id: &str, name: &str) -> Result<Instance> {
        self.update(id, |instance| instance.name = name.to_owned())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _guard = self.guard();
        let dir = self.instance_dir(id)?;
        // Detach shared folders explicitly so deletion can never reach shared content.
        detach_all(&dir.join("minecraft"))?;
        std::fs::remove_dir_all(&dir).at(&dir)
    }

    pub fn copy(&self, id: &str, new_name: &str, options: &CopyOptions) -> Result<Instance> {
        let _guard = self.guard();
        let source = self.get(id)?;
        let source_game_dir = source.game_dir(&self.paths);
        let excluded = options.excluded_entries();
        // Shared folders are re-linked for the copy instead of being duplicated.
        let linked: Vec<&str> = SharedFolder::ALL
            .iter()
            .map(|folder| folder.dir_name())
            .filter(|name| is_link(&source_game_dir.join(name)))
            .collect();

        self.stage(|staging| {
            let game_dir = staging.join("minecraft");
            copy_dir(&source_game_dir, &game_dir, &|relative| {
                let mut parts = relative.components();
                let top_level = parts.next().map(|part| part.as_os_str().to_string_lossy().to_lowercase());
                let is_top_level = parts.next().is_none();
                is_top_level
                    && top_level.is_some_and(|name| {
                        excluded.contains(&name.as_str()) || linked.contains(&name.as_str())
                    })
            })?;

            if let Some(file_name) = custom_icon_file(&source.icon) {
                let from = source.dir(&self.paths).join(file_name);
                if from.is_file() {
                    let to = staging.join(file_name);
                    std::fs::copy(&from, &to).at(&to)?;
                }
            }

            let mut instance = source.clone();
            instance.name = new_name.to_owned();
            instance.created = Utc::now();
            if !options.keep_playtime {
                instance.playtime = Playtime::default();
                instance.last_played = None;
            }
            for folder in &instance.shared_folders {
                enable_shared(&self.paths, &game_dir, *folder)?;
            }
            Ok(instance)
        })
    }

    /// Distinct group names in use, sorted.
    pub fn groups(&self) -> Result<Vec<String>> {
        let groups: BTreeSet<String> = self
            .list()?
            .instances
            .into_iter()
            .filter_map(|instance| instance.group)
            .collect();
        Ok(groups.into_iter().collect())
    }

    /// Move an instance into a group; `None` or an empty name ungroups it.
    pub fn set_group(&self, id: &str, group: Option<String>) -> Result<Instance> {
        let group = normalize_group(group);
        self.update(id, |instance| instance.group = group)
    }

    /// Set which folders the instance shares: newly chosen ones are linked (existing local content
    /// moves into the shared folder), dropped ones become real folders holding a copy.
    pub fn set_shared_folders(&self, id: &str, folders: BTreeSet<SharedFolder>) -> Result<Instance> {
        let _guard = self.guard();
        let mut instance = self.get(id)?;
        let game_dir = instance.game_dir(&self.paths);
        for folder in SharedFolder::ALL {
            let wanted = folders.contains(&folder);
            if instance.shared_folders.contains(&folder) == wanted {
                continue;
            }
            if wanted {
                enable_shared(&self.paths, &game_dir, folder)?;
                instance.shared_folders.insert(folder);
            } else {
                disable_shared(&self.paths, &game_dir, folder)?;
                instance.shared_folders.remove(&folder);
            }
            // Saved per folder, so a failure on a later one keeps the file in line with the disk.
            self.save(&instance)?;
        }
        Ok(instance)
    }

    /// Add a finished play session to the instance and to the global total. Concurrent sessions of
    /// one instance are recorded separately, so overlapping time is counted once per session.
    /// Does nothing when playtime recording is switched off in the global settings.
    pub fn record_session(&self, id: &str, started_at: DateTime<Utc>, ended_at: DateTime<Utc>) -> Result<Instance> {
        let _guard = self.guard();
        let mut instance = self.get(id)?;
        if !GlobalSettings::load(&self.paths)?.playtime.record {
            return Ok(instance);
        }

        let secs = (ended_at - started_at).num_seconds().max(0) as u64;
        instance.playtime.total_secs += secs;
        instance.playtime.last_secs = secs;
        // Sessions may overlap, so an older one can end after a newer one.
        instance.last_played = Some(instance.last_played.map_or(started_at, |last| last.max(started_at)));
        self.save(&instance)?;
        GlobalSettings::update(&self.paths, |settings| settings.playtime.total_secs += secs)?;
        Ok(instance)
    }

    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn save(&self, instance: &Instance) -> Result<()> {
        write_json_atomic(&self.instance_dir(&instance.id)?.join(INSTANCE_FILE), instance)
    }

    /// Instance directory for an id, rejecting ids that could point outside `instances/`.
    fn instance_dir(&self, id: &str) -> Result<PathBuf> {
        if dir_name_from_string(id) != id {
            return Err(Error::Other(format!("invalid instance id: {id:?}")));
        }
        Ok(self.paths.instances_dir().join(id))
    }

    /// Build an instance in `instances/.tmp/<random>`, then move it to its final directory,
    /// so an interrupted or failed creation leaves nothing behind.
    fn stage(&self, build: impl FnOnce(&Path) -> Result<Instance>) -> Result<Instance> {
        let instances_dir = self.paths.instances_dir();
        let staging = instances_dir.join(".tmp").join(&uuid::Uuid::new_v4().simple().to_string()[..12]);
        std::fs::create_dir_all(&staging).at(&staging)?;

        let committed = (|| {
            let mut instance = build(&staging)?;
            instance.id = unique_dir_name(&instances_dir, &instance.name)?;
            write_json_atomic(&staging.join(INSTANCE_FILE), &instance)?;
            let destination = instances_dir.join(&instance.id);
            std::fs::rename(&staging, &destination).at(&destination)?;
            Ok(instance)
        })();

        if committed.is_err() {
            let cleaned = detach_all(&staging.join("minecraft"))
                .and_then(|()| std::fs::remove_dir_all(&staging).at(&staging));
            if let Err(error) = cleaned {
                tracing::warn!("could not remove instance staging dir: {error}");
            }
        }
        // Succeeds only when no other staging dir is left in it.
        let _ = std::fs::remove_dir(instances_dir.join(".tmp"));
        committed
    }

    /// Startup cleanup: a staging dir that outlived its process is an interrupted build, never an instance.
    pub fn remove_leftover_staging(&self) {
        let root = self.paths.instances_dir().join(".tmp");
        let Ok(entries) = std::fs::read_dir(&root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let removed = detach_all(&path.join("minecraft")).and_then(|()| std::fs::remove_dir_all(&path).at(&path));
            if let Err(error) = removed {
                tracing::warn!("could not remove leftover instance staging dir: {error}");
            }
        }
        let _ = std::fs::remove_dir(&root);
    }
}

fn normalize_group(group: Option<String>) -> Option<String> {
    group.map(|g| g.trim().to_owned()).filter(|g| !g.is_empty())
}

/// Turn a display name into a safe directory name: path separators and characters Windows forbids
/// become `-`, leading/trailing dots and spaces go, reserved device names get a `_` prefix.
pub(super) fn dir_name_from_string(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = replaced.trim_matches(|c| c == '.' || c == ' ');
    let mut cleaned: String = trimmed.chars().take(MAX_DIR_NAME_CHARS).collect();
    cleaned = cleaned.trim_end_matches(['.', ' ']).to_owned();
    if cleaned.is_empty() {
        return "instance".to_owned();
    }
    let stem = cleaned.split('.').next().unwrap_or(&cleaned).to_uppercase();
    if WINDOWS_RESERVED_NAMES.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    cleaned
}

/// `dir_name_from_string(name)`, with ` (n)` appended while the directory already exists.
fn unique_dir_name(instances_dir: &Path, name: &str) -> Result<String> {
    let base = dir_name_from_string(name);
    if !instances_dir.join(&base).exists() {
        return Ok(base);
    }
    (2..=MAX_UNIQUE_SUFFIX)
        .map(|n| format!("{base} ({n})"))
        .find(|candidate| !instances_dir.join(candidate).exists())
        .ok_or_else(|| Error::Other(format!("no free directory name for instance {name:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> (tempfile::TempDir, InstanceManager) {
        let dir = tempfile::tempdir().unwrap();
        let manager = InstanceManager::new(Paths::new(dir.path().join("data")));
        (dir, manager)
    }

    fn new_instance(name: &str) -> NewInstance {
        NewInstance {
            name: name.to_owned(),
            icon: None,
            group: None,
            components: vec![ComponentSpec {
                uid: "net.minecraft".to_owned(),
                version: "1.21.1".to_owned(),
                dependency_only: false,
            }],
            managed_pack: None,
        }
    }

    #[test]
    fn sanitises_directory_names() {
        assert_eq!(dir_name_from_string("My/Pack\\v2"), "My-Pack-v2");
        assert_eq!(dir_name_from_string("a<b>c:d\"e|f?g*h"), "a-b-c-d-e-f-g-h");
        assert_eq!(dir_name_from_string("  ..hidden.. "), "hidden");
        assert_eq!(dir_name_from_string("con"), "_con");
        assert_eq!(dir_name_from_string("NUL.txt"), "_NUL.txt");
        assert_eq!(dir_name_from_string(".."), "instance");
        assert_eq!(dir_name_from_string(""), "instance");
        assert_eq!(dir_name_from_string("Сборка 1.21 🙂"), "Сборка 1.21 🙂");
        assert_eq!(dir_name_from_string(&"x".repeat(200)).chars().count(), MAX_DIR_NAME_CHARS);
    }

    #[test]
    fn sanitising_is_idempotent_so_ids_validate() {
        for name in ["a/b", "con", "x (2)", ".tmp", "ok"] {
            let once = dir_name_from_string(name);
            assert_eq!(dir_name_from_string(&once), once);
        }
    }

    #[test]
    fn rejects_traversal_ids() {
        let (_dir, manager) = manager();
        for id in ["..", "../x", "a/b", ".tmp", ""] {
            assert!(manager.get(id).is_err(), "{id:?}");
        }
        assert!(manager.delete("../data").is_err());
    }

    #[test]
    fn leftover_staging_is_removed_and_not_listed() {
        let (_dir, manager) = manager();
        let leftover = manager.paths().instances_dir().join(".tmp").join("abc").join("minecraft");
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join(INSTANCE_FILE), b"{}").unwrap();
        assert!(manager.list().unwrap().instances.is_empty());
        manager.remove_leftover_staging();
        assert!(!manager.paths().instances_dir().join(".tmp").exists());
    }

    #[test]
    fn create_list_get_rename_delete() {
        let (_dir, manager) = manager();
        assert!(manager.list().unwrap().instances.is_empty());

        let first = manager.create(new_instance("Vanilla")).unwrap();
        let second = manager.create(new_instance("Vanilla")).unwrap();
        assert_eq!((first.id.as_str(), second.id.as_str()), ("Vanilla", "Vanilla (2)"));
        assert!(first.game_dir(manager.paths()).is_dir());
        assert!(!manager.paths().instances_dir().join(".tmp").exists());

        let list = manager.list().unwrap();
        assert_eq!(list.instances.len(), 2);
        assert!(list.errors.is_empty());

        let renamed = manager.rename("Vanilla", "Renamed").unwrap();
        assert_eq!((renamed.id.as_str(), renamed.name.as_str()), ("Vanilla", "Renamed"));
        assert_eq!(manager.get("Vanilla").unwrap().name, "Renamed");
        assert!(manager.paths().instances_dir().join("Vanilla").is_dir());

        manager.delete("Vanilla").unwrap();
        assert!(manager.get("Vanilla").is_err());
        assert_eq!(manager.list().unwrap().instances.len(), 1);
    }

    #[test]
    fn broken_instances_are_reported_not_dropped() {
        let (_dir, manager) = manager();
        manager.create(new_instance("Good")).unwrap();
        let broken = manager.paths().instances_dir().join("Bad");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(INSTANCE_FILE), "{nope").unwrap();
        std::fs::create_dir_all(manager.paths().instances_dir().join("not-an-instance")).unwrap();

        let list = manager.list().unwrap();
        assert_eq!(list.instances.len(), 1);
        assert_eq!(list.errors.len(), 1);
        assert_eq!(list.errors[0].id, "Bad");
    }

    #[test]
    fn groups() {
        let (_dir, manager) = manager();
        manager.create(new_instance("A")).unwrap();
        let mut grouped = new_instance("B");
        grouped.group = Some(" Modded ".to_owned());
        manager.create(grouped).unwrap();
        assert_eq!(manager.groups().unwrap(), vec!["Modded".to_owned()]);

        manager.set_group("A", Some("Alpha".to_owned())).unwrap();
        manager.set_group("B", Some(String::new())).unwrap();
        assert_eq!(manager.groups().unwrap(), vec!["Alpha".to_owned()]);
    }

    #[test]
    fn playtime_is_recorded_per_instance_and_globally() {
        let (_dir, manager) = manager();
        manager.create(new_instance("A")).unwrap();
        let start = Utc::now();

        manager.record_session("A", start, start + chrono::Duration::seconds(90)).unwrap();
        let instance = manager
            .record_session("A", start, start + chrono::Duration::seconds(30))
            .unwrap();
        assert_eq!((instance.playtime.total_secs, instance.playtime.last_secs), (120, 30));
        assert_eq!(instance.last_played, Some(start));
        assert_eq!(GlobalSettings::load(manager.paths()).unwrap().playtime.total_secs, 120);

        // A clock that went backwards must not subtract time.
        let instance = manager.record_session("A", start, start - chrono::Duration::seconds(5)).unwrap();
        assert_eq!((instance.playtime.total_secs, instance.playtime.last_secs), (120, 0));
    }

    #[test]
    fn playtime_recording_can_be_disabled() {
        let (_dir, manager) = manager();
        manager.create(new_instance("A")).unwrap();
        GlobalSettings::update(manager.paths(), |s| s.playtime.record = false).unwrap();
        let start = Utc::now();
        let instance = manager.record_session("A", start, start + chrono::Duration::seconds(60)).unwrap();
        assert_eq!(instance.playtime, Playtime::default());
        assert_eq!(GlobalSettings::load(manager.paths()).unwrap().playtime.total_secs, 0);
    }

    mod files {
        use super::*;

        fn write(path: PathBuf, content: &str) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        #[test]
        fn copy_applies_filters_and_playtime_choice() {
            let (_dir, manager) = manager();
            let source = manager.create(new_instance("Source")).unwrap();
            let game = source.game_dir(manager.paths());
            for file in ["saves/w/level.dat", "mods/a.jar", "config/c.toml", "screenshots/s.png", "options.txt", "servers.dat", "shaderpacks/s.zip", "logs/latest.log"] {
                write(game.join(file), file);
            }
            let start = Utc::now();
            manager.record_session("Source", start, start + chrono::Duration::seconds(60)).unwrap();

            let options = CopyOptions {
                copy_saves: false,
                copy_mods: false,
                copy_screenshots: false,
                keep_playtime: false,
                ..CopyOptions::default()
            };
            let copy = manager.copy("Source", "Copy", &options).unwrap();
            let copy_game = copy.game_dir(manager.paths());
            for kept in ["options.txt", "servers.dat", "shaderpacks/s.zip", "logs/latest.log"] {
                assert!(copy_game.join(kept).is_file(), "{kept}");
            }
            for dropped in ["saves", "mods", "config", "screenshots"] {
                assert!(!copy_game.join(dropped).exists(), "{dropped}");
            }
            assert_eq!(copy.playtime, Playtime::default());
            assert_eq!(copy.last_played, None);
            assert_eq!(manager.get("Source").unwrap().playtime.total_secs, 60);

            let full = manager.copy("Source", "Full", &CopyOptions::default()).unwrap();
            assert!(full.game_dir(manager.paths()).join("saves/w/level.dat").is_file());
            assert_eq!(full.playtime.total_secs, 60);
        }

        #[test]
        fn new_instances_share_the_default_folders() {
            let (_dir, manager) = manager();
            let paths = manager.paths();
            let first = manager.create(new_instance("A")).unwrap();
            let game = first.game_dir(paths);
            assert!(is_link(&game.join("resourcepacks")) && is_link(&game.join("shaderpacks")));
            assert!(!is_link(&game.join("screenshots")));

            GlobalSettings::update(paths, |s| s.shared_folders_default = BTreeSet::from([SharedFolder::Screenshots]))
                .unwrap();
            let second = manager.create(new_instance("B")).unwrap();
            assert_eq!(second.shared_folders, BTreeSet::from([SharedFolder::Screenshots]));
            assert!(is_link(&second.game_dir(paths).join("screenshots")));
            assert!(!second.game_dir(paths).join("resourcepacks").exists());
        }

        #[test]
        fn shared_folders_are_relinked_in_copies_and_survive_delete() {
            let (_dir, manager) = manager();
            let paths = manager.paths();
            let source = manager.create(new_instance("Source")).unwrap();
            manager
                .set_shared_folders("Source", SharedFolder::ALL.into_iter().collect())
                .unwrap();
            write(paths.shared_dir().join("resourcepacks/p.zip"), "pack");
            write(source.game_dir(paths).join("screenshots/s.png"), "shot");
            let source_packs = source.game_dir(paths).join("resourcepacks");
            assert_eq!(std::fs::read(source_packs.join("p.zip")).unwrap(), b"pack");

            let copy = manager.copy("Source", "Copy", &CopyOptions::default()).unwrap();
            assert_eq!(copy.shared_folders.len(), 4);
            for folder in SharedFolder::ALL.into_iter().filter(|folder| folder.is_linked()) {
                assert!(is_link(&copy.game_dir(paths).join(folder.dir_name())), "{folder:?}");
            }
            assert!(copy.game_dir(paths).join("screenshots/s.png").is_file());

            manager.delete("Source").unwrap();
            manager.delete("Copy").unwrap();
            assert!(paths.shared_dir().join("resourcepacks/p.zip").is_file());
            assert!(paths.shared_dir().join("screenshots/s.png").is_file());
        }

        #[test]
        fn set_shared_folders_applies_the_difference() {
            let (_dir, manager) = manager();
            let paths = manager.paths();
            let instance = manager.create(new_instance("A")).unwrap();
            let game = instance.game_dir(paths);
            write(paths.shared_dir().join("shaderpacks/s.zip"), "shader");
            write(game.join("screenshots/a.png"), "mine");
            write(paths.shared_dir().join("screenshots/a.png"), "other");

            let updated = manager
                .set_shared_folders("A", BTreeSet::from([SharedFolder::Resourcepacks, SharedFolder::Screenshots]))
                .unwrap();
            assert_eq!(updated, manager.get("A").unwrap());
            assert!(is_link(&game.join("resourcepacks")) && is_link(&game.join("screenshots")));
            // Local content moved into the shared folder; the clash was renamed, nothing overwritten.
            assert_eq!(std::fs::read(paths.shared_dir().join("screenshots/a.png")).unwrap(), b"other");
            assert_eq!(std::fs::read(paths.shared_dir().join("screenshots/a (1).png")).unwrap(), b"mine");
            // Dropped folder is a real folder with a copy; the shared one stays intact.
            assert!(!is_link(&game.join("shaderpacks")));
            assert!(game.join("shaderpacks/s.zip").is_file());
            assert!(paths.shared_dir().join("shaderpacks/s.zip").is_file());

            let none = manager.set_shared_folders("A", BTreeSet::new()).unwrap();
            assert!(none.shared_folders.is_empty());
            assert!(manager.get("A").unwrap().shared_folders.is_empty());
            assert!(!is_link(&game.join("resourcepacks")) && game.join("resourcepacks").is_dir());
        }

        #[test]
        fn legacy_instance_file_keeps_its_meaning() {
            let (_dir, manager) = manager();
            let paths = manager.paths();
            let instance = manager.create(new_instance("A")).unwrap();
            let file = instance.dir(paths).join(INSTANCE_FILE);
            let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            json.as_object_mut().unwrap().remove("sharedFolders");
            json["shareResourcepacks"] = true.into();
            std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
            assert_eq!(
                manager.get("A").unwrap().shared_folders,
                BTreeSet::from([SharedFolder::Resourcepacks])
            );
        }
    }
}
