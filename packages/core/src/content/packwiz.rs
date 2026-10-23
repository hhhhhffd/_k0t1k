//! packwiz `.index/<slug>.pw.toml` files, written the way Prism does so instances stay compatible
//! with Prism and packwiz tooling.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{DependencyKind, Platform, Project, ProjectVersion, ResourceKind, Side, VersionFile};
use crate::error::{Error, IoContext, Result};

const INDEX_SUFFIX: &str = ".pw.toml";

/// Reading is tolerant: every missing key falls back to its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwMod {
    pub name: String,
    pub filename: String,
    /// `client`, `server`, `both` or empty.
    pub side: String,
    #[serde(rename = "x-prismlauncher-loaders")]
    pub loaders: Vec<String>,
    #[serde(rename = "x-prismlauncher-mc-versions")]
    pub mc_versions: Vec<String>,
    #[serde(rename = "x-prismlauncher-release-type")]
    pub release_type: String,
    #[serde(rename = "x-prismlauncher-version-number")]
    pub version_number: String,
    #[serde(rename = "x-prismlauncher-lock-update")]
    pub lock_update: bool,
    #[serde(rename = "x-prismlauncher-dependencies")]
    pub dependencies: Vec<PwDependency>,
    pub download: PwDownload,
    pub update: PwUpdate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwDependency {
    #[serde(rename = "addonId")]
    pub addon_id: String,
    /// `REQUIRED`, `OPTIONAL`, `EMBEDDED` or `INCOMPATIBLE`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwDownload {
    /// `url`, or `metadata:curseforge` (the URL is then resolved through the API).
    pub mode: String,
    pub url: String,
    #[serde(rename = "hash-format")]
    pub hash_format: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<PwModrinth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<PwCurseForge>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwModrinth {
    #[serde(rename = "mod-id")]
    pub mod_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PwCurseForge {
    #[serde(rename = "file-id")]
    pub file_id: u64,
    #[serde(rename = "project-id")]
    pub project_id: u64,
}

/// An index file found on disk.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub data: PwMod,
}

impl PwMod {
    pub fn new(
        project: &Project,
        version: &ProjectVersion,
        file: &VersionFile,
        lock_update: bool,
    ) -> Result<Self> {
        let (hash_format, hash) = match (&file.hashes.sha512, &file.hashes.sha1) {
            (Some(hash), _) => ("sha512", hash),
            (None, Some(hash)) => ("sha1", hash),
            (None, None) => {
                return Err(Error::Other(format!("{} has no known hash", file.filename)));
            }
        };
        let (mode, url, update) = match version.platform {
            Platform::Modrinth => (
                "url",
                file.url.clone().unwrap_or_default(),
                PwUpdate {
                    modrinth: Some(PwModrinth {
                        mod_id: version.project_id.clone(),
                        version: version.id.clone(),
                    }),
                    curseforge: None,
                },
            ),
            Platform::CurseForge => {
                let number = |value: &str| {
                    value
                        .parse()
                        .map_err(|_| Error::Other(format!("{value:?} is not a CurseForge id")))
                };
                (
                    "metadata:curseforge",
                    String::new(),
                    PwUpdate {
                        modrinth: None,
                        curseforge: Some(PwCurseForge {
                            file_id: number(&version.id)?,
                            project_id: number(&version.project_id)?,
                        }),
                    },
                )
            }
        };

        let mut mc_versions = version.game_versions.clone();
        mc_versions.sort_by(|a, b| compare_versions(a, b));
        mc_versions.dedup();

        Ok(PwMod {
            name: project.title.clone(),
            filename: file.filename.clone(),
            side: match project.side {
                Some(Side::Client) => "client",
                Some(Side::Server) => "server",
                Some(Side::Both) => "both",
                None => "",
            }
            .to_owned(),
            loaders: version
                .loaders
                .iter()
                .map(|loader| loader.modrinth_id().to_owned())
                .collect(),
            mc_versions,
            release_type: version.release_type.as_str().to_owned(),
            version_number: version.version_number.clone(),
            lock_update,
            dependencies: version
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    let kind = match dependency.kind {
                        DependencyKind::Required => "REQUIRED",
                        DependencyKind::Optional => "OPTIONAL",
                        DependencyKind::Embedded => "EMBEDDED",
                        DependencyKind::Incompatible => "INCOMPATIBLE",
                    };
                    Some(PwDependency {
                        addon_id: dependency.project_id.clone()?,
                        kind: kind.to_owned(),
                        version: dependency.version_id.clone(),
                    })
                })
                .collect(),
            download: PwDownload {
                mode: mode.to_owned(),
                url,
                hash_format: hash_format.to_owned(),
                hash: hash.clone(),
            },
            update,
        })
    }

    /// Platform, project id and version/file id this file was installed from.
    pub fn origin(&self) -> Option<(Platform, String, String)> {
        if let Some(curseforge) = &self.update.curseforge {
            return Some((
                Platform::CurseForge,
                curseforge.project_id.to_string(),
                curseforge.file_id.to_string(),
            ));
        }
        let modrinth = self.update.modrinth.as_ref()?;
        Some((
            Platform::Modrinth,
            modrinth.mod_id.clone(),
            modrinth.version.clone(),
        ))
    }
}

/// Numeric-aware comparison of Minecraft versions (`1.9` < `1.21`); unparsable parts compare as text.
fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |version: &'_ str| {
        version
            .split(['.', '-'])
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(a), parts(b));
    for (left, right) in a.iter().zip(&b) {
        let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

/// Directory holding the `.pw.toml` files. Prism keeps shader pack indexes in the folder itself.
pub fn index_dir(game_dir: &Path, kind: ResourceKind) -> PathBuf {
    let folder = game_dir.join(kind.folder());
    match kind {
        ResourceKind::ShaderPack => folder,
        _ => folder.join(".index"),
    }
}

pub fn read(path: &Path) -> Result<PwMod> {
    let text = std::fs::read_to_string(path).at(path)?;
    toml::from_str(&text)
        .map_err(|error| Error::Other(format!("invalid packwiz index {}: {error}", path.display())))
}

/// Writes `<slug>.pw.toml`, replacing any existing index of that slug (compared case-insensitively).
pub fn write(index_dir: &Path, slug: &str, data: &PwMod) -> Result<PathBuf> {
    if slug.is_empty() || slug.starts_with('.') || slug.contains(['/', '\\']) {
        return Err(Error::Other(format!(
            "{slug:?} is not usable as an index file name"
        )));
    }
    let text = toml::to_string(data)
        .map_err(|error| Error::Other(format!("cannot serialize packwiz index: {error}")))?;
    std::fs::create_dir_all(index_dir).at(index_dir)?;
    let file_name = format!("{slug}{INDEX_SUFFIX}");
    for entry in list(index_dir)? {
        if entry
            .path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(&file_name))
        {
            std::fs::remove_file(&entry.path).at(&entry.path)?;
        }
    }
    let path = index_dir.join(file_name);
    std::fs::write(&path, text).at(&path)?;
    Ok(path)
}

/// All readable index files of a directory; a missing directory is an empty index.
pub fn list(index_dir: &Path) -> Result<Vec<IndexEntry>> {
    let entries = match std::fs::read_dir(index_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(index_dir),
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.at(index_dir)?.path();
        let is_index = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(INDEX_SUFFIX));
        if is_index {
            found.push(IndexEntry {
                data: read(&path)?,
                path,
            });
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

pub fn find_by_project(
    index_dir: &Path,
    platform: Platform,
    project_id: &str,
) -> Result<Option<IndexEntry>> {
    Ok(list(index_dir)?.into_iter().find(|entry| {
        entry
            .data
            .origin()
            .is_some_and(|(p, id, _)| p == platform && id == project_id)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRISM_SAMPLE: &str = r#"
name = "Sodium"
filename = "sodium-fabric-0.5.13+mc1.20.1.jar"
side = "client"
x-prismlauncher-loaders = ["fabric", "quilt"]
x-prismlauncher-mc-versions = ["1.20.1"]
x-prismlauncher-release-type = "release"
x-prismlauncher-version-number = "mc1.20.1-0.5.13-fabric"
x-prismlauncher-lock-update = false
x-prismlauncher-dependencies = [ { addonId = "P7dR8mSH", type = "REQUIRED", version = "abc" }, { addonId = "x", type = "OPTIONAL" } ]

[download]
mode = "url"
url = "https://cdn.modrinth.com/data/AANobbMI/versions/OihdIimA/sodium.jar"
hash-format = "sha512"
hash = "81c64f9c"

[update.modrinth]
mod-id = "AANobbMI"
version = "OihdIimA"
"#;

    #[test]
    fn reads_prism_written_file_and_round_trips() {
        let parsed: PwMod = toml::from_str(PRISM_SAMPLE).unwrap();
        assert_eq!(parsed.loaders, ["fabric", "quilt"]);
        assert_eq!(parsed.dependencies[0].version.as_deref(), Some("abc"));
        assert_eq!(parsed.dependencies[1].version, None);
        assert_eq!(
            parsed.origin(),
            Some((Platform::Modrinth, "AANobbMI".into(), "OihdIimA".into()))
        );

        let text = toml::to_string(&parsed).unwrap();
        assert!(text.contains("x-prismlauncher-mc-versions = [\"1.20.1\"]"));
        assert!(text.contains("hash-format = \"sha512\""));
        assert_eq!(toml::from_str::<PwMod>(&text).unwrap(), parsed);
    }

    #[test]
    fn curseforge_origin_wins_and_missing_keys_default() {
        let parsed: PwMod = toml::from_str(
            "filename = \"a.jar\"\n[update.curseforge]\nfile-id = 7\nproject-id = 3\n[update.modrinth]\nmod-id = \"m\"\nversion = \"v\"\n",
        )
        .unwrap();
        assert_eq!(
            parsed.origin(),
            Some((Platform::CurseForge, "3".into(), "7".into()))
        );
        assert!(!parsed.lock_update);
        assert!(parsed.loaders.is_empty());
    }

    #[test]
    fn write_replaces_case_insensitively_and_list_finds_by_project() {
        let dir = std::env::temp_dir().join(format!("k0t1k-pw-{}", uuid::Uuid::new_v4()));
        let data: PwMod = toml::from_str(PRISM_SAMPLE).unwrap();
        write(&dir, "Sodium", &data).unwrap();
        write(&dir, "sodium", &data).unwrap();
        assert_eq!(list(&dir).unwrap().len(), 1);
        let found = find_by_project(&dir, Platform::Modrinth, "AANobbMI")
            .unwrap()
            .unwrap();
        assert_eq!(found.data, data);
        assert!(
            find_by_project(&dir, Platform::CurseForge, "AANobbMI")
                .unwrap()
                .is_none()
        );
        assert!(write(&dir, "../evil", &data).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn versions_sort_numerically() {
        let mut versions = vec!["1.21.4".to_owned(), "1.9".to_owned(), "1.21".to_owned()];
        versions.sort_by(|a, b| compare_versions(a, b));
        assert_eq!(versions, ["1.9", "1.21", "1.21.4"]);
    }
}
