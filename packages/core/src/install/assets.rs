use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::download::DownloadItem;
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

const RESOURCES_URL: &str = "https://resources.download.minecraft.net";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct RawAssetIndex {
    objects: BTreeMap<String, AssetObject>,
    #[serde(rename = "virtual", default)]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

/// A parsed `assets/indexes/<id>.json`.
#[derive(Debug, Clone)]
pub struct AssetIndex {
    pub id: String,
    pub objects: BTreeMap<String, AssetObject>,
    /// Old index: objects are copied to `assets/virtual/<id>/<name>`.
    pub is_virtual: bool,
    /// Older index: objects are copied to `<game dir>/resources/<name>`.
    pub map_to_resources: bool,
}

impl AssetIndex {
    /// The `${game_assets}` launch token: the reconstructed asset tree used by pre-1.7 versions.
    pub fn game_assets_dir(&self, paths: &Paths, game_dir: &Path) -> PathBuf {
        if self.map_to_resources && !self.is_virtual {
            game_dir.join("resources")
        } else {
            paths.assets_dir().join("virtual").join(&self.id)
        }
    }
}

pub(super) fn index_path(paths: &Paths, id: &str) -> Result<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\']) || id == ".." {
        return Err(Error::Other(format!("invalid asset index id `{id}`")));
    }
    Ok(paths.assets_dir().join("indexes").join(format!("{id}.json")))
}

fn object_path(paths: &Paths, hash: &str) -> Result<PathBuf> {
    if hash.len() < 2 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Other(format!("invalid asset hash `{hash}`")));
    }
    Ok(paths.assets_dir().join("objects").join(&hash[..2]).join(hash))
}

pub async fn load_asset_index(paths: &Paths, id: &str) -> Result<AssetIndex> {
    let path = index_path(paths, id)?;
    let bytes = tokio::fs::read(&path).await.at(&path)?;
    let raw: RawAssetIndex = serde_json::from_slice(&bytes)?;
    Ok(AssetIndex {
        id: id.to_owned(),
        objects: raw.objects,
        is_virtual: raw.is_virtual,
        map_to_resources: raw.map_to_resources,
    })
}

/// Download items for the index objects that are not on disk with the right size (each hash once).
pub(super) async fn missing_object_items(paths: &Paths, index: &AssetIndex) -> Result<Vec<DownloadItem>> {
    let unique: BTreeMap<&str, u64> =
        index.objects.values().map(|object| (object.hash.as_str(), object.size)).collect();

    let mut items = Vec::new();
    for (hash, size) in unique {
        let path = object_path(paths, hash)?;
        if super::is_present(&path, Some(size)).await? {
            continue;
        }
        items.push(DownloadItem {
            url: format!("{RESOURCES_URL}/{}/{hash}", &hash[..2]),
            path,
            sha1: Some(hash.to_ascii_lowercase()),
        });
    }
    Ok(items)
}

/// Copies objects to their readable names for old indexes (`virtual` and `map_to_resources`).
/// Existing files are left alone and leftovers are never deleted.
pub async fn reconstruct_assets(paths: &Paths, index: &AssetIndex, game_dir: &Path) -> Result<()> {
    if !index.is_virtual && !index.map_to_resources {
        return Ok(());
    }
    let mut targets = Vec::new();
    if index.is_virtual {
        targets.push(paths.assets_dir().join("virtual").join(&index.id));
    }
    if index.map_to_resources {
        targets.push(game_dir.join("resources"));
    }
    let mut copies = Vec::new();
    for (name, object) in &index.objects {
        let relative = Path::new(name);
        if !relative.components().all(|part| matches!(part, Component::Normal(_))) {
            return Err(Error::Other(format!("invalid asset name `{name}`")));
        }
        let source = object_path(paths, &object.hash)?;
        for target in &targets {
            copies.push((source.clone(), target.join(relative)));
        }
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        for (source, target) in copies {
            if target.exists() {
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::copy(&source, &target).at(&target)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| Error::Other(format!("asset reconstruction task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(json: &str) -> AssetIndex {
        let raw: RawAssetIndex = serde_json::from_str(json).unwrap();
        AssetIndex {
            id: "legacy".to_owned(),
            objects: raw.objects,
            is_virtual: raw.is_virtual,
            map_to_resources: raw.map_to_resources,
        }
    }

    #[tokio::test]
    async fn object_items_deduplicate_hashes_and_skip_present_files() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path());
        let index = index(
            r#"{"objects":{"a/one.ogg":{"hash":"abcdef","size":3},"b/two.ogg":{"hash":"abcdef","size":3},
                           "c.txt":{"hash":"123456","size":2}}}"#,
        );
        let items = missing_object_items(&paths, &index).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://resources.download.minecraft.net/12/123456");

        let present = object_path(&paths, "abcdef").unwrap();
        std::fs::create_dir_all(present.parent().unwrap()).unwrap();
        std::fs::write(&present, b"xyz").unwrap();
        let missing = missing_object_items(&paths, &index).await.unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].sha1.as_deref(), Some("123456"));
    }

    #[tokio::test]
    async fn reconstructs_virtual_and_resources_trees() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path().join("data"));
        let game_dir = root.path().join("game");
        let mut index =
            index(r#"{"virtual":true,"map_to_resources":true,"objects":{"sounds/a.ogg":{"hash":"abcdef","size":3}}}"#);
        let object = object_path(&paths, "abcdef").unwrap();
        std::fs::create_dir_all(object.parent().unwrap()).unwrap();
        std::fs::write(&object, b"xyz").unwrap();

        reconstruct_assets(&paths, &index, &game_dir).await.unwrap();
        let virtual_copy = paths.assets_dir().join("virtual/legacy/sounds/a.ogg");
        assert_eq!(std::fs::read(virtual_copy).unwrap(), b"xyz");
        assert_eq!(std::fs::read(game_dir.join("resources/sounds/a.ogg")).unwrap(), b"xyz");

        assert!(index.game_assets_dir(&paths, &game_dir).ends_with("virtual/legacy"));
        index.is_virtual = false;
        assert_eq!(index.game_assets_dir(&paths, &game_dir), game_dir.join("resources"));
    }

    #[tokio::test]
    async fn reconstruct_rejects_path_traversal_names() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path());
        let index = index(r#"{"virtual":true,"objects":{"../evil":{"hash":"abcdef","size":3}}}"#);
        assert!(reconstruct_assets(&paths, &index, root.path()).await.is_err());
    }

    #[test]
    fn unsafe_ids_and_hashes_are_rejected() {
        let paths = Paths::new("/data");
        assert!(index_path(&paths, "../x").is_err());
        assert!(object_path(&paths, "../../etc").is_err());
        assert!(object_path(&paths, "a").is_err());
    }
}
