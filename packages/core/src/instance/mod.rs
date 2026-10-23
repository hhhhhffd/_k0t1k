//! Instances: `instances/<id>/instance.json` plus the `minecraft/` game dir and `sessions/<id>/` (natives of a running launch).

pub mod archive;
pub(crate) mod fsutil;
mod links;
mod manager;
mod nbt;
pub mod servers;
pub mod worlds;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use links::{SharedFolder, disable_shared, enable_shared, exists_in_linked_shared_folder};
pub use manager::{BrokenInstance, CopyOptions, InstanceList, InstanceManager, NewInstance};

use crate::component::ComponentSpec;
use crate::error::Result;
use crate::paths::Paths;
use crate::settings::{CommandSettings, GlobalSettings, JavaSettings, MemorySettings, WindowSettings};

pub const INSTANCE_FILE: &str = "instance.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playtime {
    pub total_secs: u64,
    pub last_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManagedPlatform {
    Modrinth,
    CurseForge,
}

/// The modpack an instance was installed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPack {
    pub platform: ManagedPlatform,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version_name: String,
}

/// Per-instance overrides. `None` inherits the global value; `Some` replaces the whole group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InstanceSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java: Option<JavaSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemorySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jvm_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<CommandSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}

/// Instance overrides merged over the global settings; what a launch actually uses.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveSettings {
    pub java: JavaSettings,
    pub memory: MemorySettings,
    pub jvm_args: String,
    pub window: WindowSettings,
    pub commands: CommandSettings,
    pub env: BTreeMap<String, String>,
}

impl InstanceSettings {
    pub fn resolve(&self, global: &GlobalSettings) -> EffectiveSettings {
        EffectiveSettings {
            java: self.java.clone().unwrap_or_else(|| global.java.clone()),
            memory: self.memory.clone().unwrap_or_else(|| global.memory.clone()),
            jvm_args: self.jvm_args.clone().unwrap_or_else(|| global.jvm_args.clone()),
            window: self.window.clone().unwrap_or_else(|| global.window.clone()),
            commands: self.commands.clone().unwrap_or_else(|| global.commands.clone()),
            env: self.env.clone().unwrap_or_else(|| global.env.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    /// Directory name under `instances/`; not stored in the file.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub group: Option<String>,
    pub components: Vec<ComponentSpec>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
    #[serde(default)]
    pub playtime: Playtime,
    #[serde(default)]
    pub managed_pack: Option<ManagedPack>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub settings: InstanceSettings,
    /// Game folders linked to `shared/<folder>`, shared with every other instance that links them.
    /// Old files had `shareResourcepacks: bool`; `Instance::from_json` converts it.
    #[serde(default)]
    pub shared_folders: BTreeSet<SharedFolder>,
}

fn default_icon() -> String {
    "default".to_owned()
}

impl Instance {
    /// Parse `instance.json`. The legacy `shareResourcepacks` flag becomes `sharedFolders`
    /// (true: only `resourcepacks`, false: none) unless the new field is present.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
        if let Some(object) = value.as_object_mut() {
            let legacy = object.remove("shareResourcepacks");
            if !object.contains_key("sharedFolders") {
                let folders = match legacy.and_then(|flag| flag.as_bool()) {
                    Some(true) => vec![SharedFolder::Resourcepacks],
                    _ => Vec::new(),
                };
                object.insert("sharedFolders".to_owned(), serde_json::to_value(folders)?);
            }
        }
        Ok(serde_json::from_value(value)?)
    }


    pub fn dir(&self, paths: &Paths) -> PathBuf {
        paths.instances_dir().join(&self.id)
    }

    pub fn game_dir(&self, paths: &Paths) -> PathBuf {
        self.dir(paths).join("minecraft")
    }

    /// Per-launch files (`<session>/natives` and the JVM argfile next to it), so concurrent
    /// launches of one instance do not touch each other's files.
    pub fn sessions_dir(&self, paths: &Paths) -> PathBuf {
        self.dir(paths).join("sessions")
    }

    pub fn effective_settings(&self, global: &GlobalSettings) -> EffectiveSettings {
        self.settings.resolve(global)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_replace_whole_groups_and_none_inherits() {
        let global = GlobalSettings {
            jvm_args: "-Xss1M".to_owned(),
            ..GlobalSettings::default()
        };

        let inherited = InstanceSettings::default().resolve(&global);
        assert_eq!(inherited.jvm_args, "-Xss1M");
        assert_eq!(inherited.memory, global.memory);
        assert_eq!(inherited.window, global.window);

        let overridden = InstanceSettings {
            memory: Some(MemorySettings {
                min_mib: 1024,
                max_mib: 8192,
            }),
            jvm_args: Some(String::new()),
            java: Some(JavaSettings {
                path: Some("/opt/jdk/bin/java".into()),
                ..JavaSettings::default()
            }),
            ..InstanceSettings::default()
        }
        .resolve(&global);
        assert_eq!(overridden.memory.max_mib, 8192);
        // An explicit empty value overrides; it is not the same as inheriting.
        assert_eq!(overridden.jvm_args, "");
        assert_eq!(overridden.java.path, Some("/opt/jdk/bin/java".into()));
        assert_eq!(overridden.window, global.window);
    }

    #[test]
    fn minimal_file_loads_with_defaults() {
        let instance: Instance = serde_json::from_str(
            r#"{"name":"A","components":[{"uid":"net.minecraft","version":"1.21.1"}],"created":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert!(instance.shared_folders.is_empty());
        assert_eq!(instance.icon, "default");
        assert_eq!(instance.settings, InstanceSettings::default());
        assert_eq!(instance.playtime, Playtime::default());
    }

    #[test]
    fn legacy_share_flag_migrates_and_new_field_wins() {
        let load = |extra: &str| {
            Instance::from_json(
                format!(r#"{{"name":"A","components":[],"created":"2026-01-01T00:00:00Z"{extra}}}"#).as_bytes(),
            )
            .unwrap()
            .shared_folders
        };
        assert_eq!(load(r#","shareResourcepacks":true"#), BTreeSet::from([SharedFolder::Resourcepacks]));
        assert!(load(r#","shareResourcepacks":false"#).is_empty());
        assert_eq!(
            load(r#","shareResourcepacks":false,"sharedFolders":["screenshots","shaderpacks"]"#),
            BTreeSet::from([SharedFolder::Screenshots, SharedFolder::Shaderpacks])
        );
        assert!(load(r#","shareResourcepacks":true,"sharedFolders":[]"#).is_empty());
    }

    #[test]
    fn unset_overrides_are_not_written() {
        let json = serde_json::to_string(&InstanceSettings::default()).unwrap();
        assert_eq!(json, "{}");
    }
}
