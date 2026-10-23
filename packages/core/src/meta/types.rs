use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::profile::library::Library;

#[derive(Debug, Clone, Deserialize)]
pub struct PackageIndex {
    pub packages: Vec<PackageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageEntry {
    pub uid: String,
    /// Hash of the package's `index.json`.
    pub sha256: String,
}

/// A dependency (`requires`) or incompatibility (`conflicts`) of a version.
/// `equals` is a hard pin, `suggests` a soft default, neither means "any version".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub uid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggests: Option<String>,
}

/// One entry of `<uid>/index.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    pub version: String,
    /// `release`, `snapshot`, `old_beta` or `old_alpha` for Minecraft; usually absent for loaders.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    pub release_time: DateTime<Utc>,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    #[serde(default)]
    pub conflicts: Vec<Requirement>,
    #[serde(default)]
    pub volatile: bool,
    /// Hash of `<uid>/<version>.json`.
    #[serde(skip_serializing)]
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionList {
    pub uid: String,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoggingConfig {
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub url: String,
}

/// A Prism meta version file (the "OneSix" format).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionFile {
    pub uid: String,
    pub version: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub volatile: bool,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub maven_files: Vec<Library>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub main_jar: Option<Library>,
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "+jvmArgs", default)]
    pub jvm_args: Vec<String>,
    #[serde(rename = "+tweakers", default)]
    pub tweakers: Vec<String>,
    #[serde(rename = "+traits", default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub compatible_java_majors: Vec<u32>,
    #[serde(default)]
    pub compatible_java_name: Option<String>,
    #[serde(default)]
    pub logging: Option<LoggingConfig>,
}
