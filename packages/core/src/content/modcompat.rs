//! Does an installed mod jar declare itself for the instance's Minecraft version and loader?
//! Only what the jar's own descriptor states is checked; anything unreadable or not understood is
//! "unknown" and never reported, so a warning always has a stated reason behind it.
//! All functions here do blocking file I/O; call them from `spawn_blocking` in async code.

use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Loader, ResourceKind};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::error::{Error, IoContext, Result};

const CACHE_FILE: &str = "modcompat.json";

/// What one descriptor file of a jar says. `minecraft` holds alternatives (any may match); it is
/// empty when the descriptor states nothing about Minecraft. Fabric and Quilt use semver
/// predicates, Forge and NeoForge maven ranges, legacy `mcmod.info` becomes a `1.12.x` wildcard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub loader: Loader,
    pub minecraft: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModCompat {
    /// Name on disk, including a trailing `.disabled`.
    pub file_name: String,
    pub enabled: bool,
    /// What the mod asks of Minecraft, like `>=1.20 <1.21`; set only for a version mismatch.
    pub needs: Option<String>,
    pub minecraft_mismatch: bool,
    /// Loaders the jar is made for; set only for a loader mismatch.
    pub made_for_loaders: Vec<Loader>,
    pub loader_mismatch: bool,
}

/// The mod loader among an instance's components, if any. A turned-off loader counts as none:
/// the game starts as vanilla and mods are not loaded at all.
pub fn loader_of_components(components: &[ComponentSpec]) -> Option<Loader> {
    components.iter().filter(|component| !component.disabled).find_map(|component| match component.uid.as_str() {
        "net.fabricmc.fabric-loader" => Some(Loader::Fabric),
        "org.quiltmc.quilt-loader" => Some(Loader::Quilt),
        "net.minecraftforge" => Some(Loader::Forge),
        "net.neoforged" => Some(Loader::NeoForge),
        _ => None,
    })
}

/// The Minecraft version among an instance's components.
pub fn minecraft_of_components(components: &[ComponentSpec]) -> Option<&str> {
    components.iter().find(|component| component.uid == MINECRAFT_UID).map(|component| component.version.as_str())
}

/// Mod jars of the instance that do not fit its Minecraft version or loader. Descriptors are cached
/// in `<instance>/modcompat.json` by file name, size and modification time.
pub fn check_mods(game_dir: &Path, instance_dir: &Path, minecraft: &str, loader: Option<Loader>) -> Result<Vec<ModCompat>> {
    let Some(loader) = loader else {
        return Ok(Vec::new());
    };
    let folder = game_dir.join(ResourceKind::Mod.folder());
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(&folder),
    };
    let cache_path = instance_dir.join(CACHE_FILE);
    let old_cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, Vec<Descriptor>>>(&bytes).ok())
        .unwrap_or_default();
    let mut cache = BTreeMap::new();
    let mut jars = Vec::new();
    for entry in entries {
        let entry = entry.at(&folder)?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let lower = file_name.to_ascii_lowercase();
        if !(lower.ends_with(".jar") || lower.ends_with(".jar.disabled")) || !entry.file_type().at(&path)?.is_file() {
            continue;
        }
        let meta = entry.metadata().at(&path)?;
        let modified = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let key = format!("{file_name}|{}|{modified}", meta.len());
        let descriptors = match old_cache.get(&key) {
            Some(cached) => cached.clone(),
            None => match std::fs::File::open(&path).at(&path).and_then(descriptors_of_jar) {
                Ok(descriptors) => descriptors,
                Err(error) => {
                    // not cached, so a jar that was locked or half-copied is read again next time
                    tracing::warn!("cannot read the descriptors of {}: {error}", path.display());
                    jars.push((file_name, Vec::new()));
                    continue;
                }
            },
        };
        cache.insert(key, descriptors.clone());
        jars.push((file_name, descriptors));
    }
    if cache != old_cache {
        match serde_json::to_vec(&cache) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(&cache_path, bytes) {
                    tracing::warn!("cannot write {}: {error}", cache_path.display());
                }
            }
            Err(error) => tracing::warn!("cannot serialize the mod compatibility cache: {error}"),
        }
    }

    // A Fabric jar can run on Forge through Sinytra Connector; with it around, loaders are not judged.
    let has_connector = jars.iter().any(|(name, _)| name.to_ascii_lowercase().contains("connector"));
    let mut found: Vec<ModCompat> = jars
        .into_iter()
        .filter_map(|(file_name, descriptors)| judge(file_name, &descriptors, minecraft, loader, has_connector))
        .collect();
    found.sort_by_key(|compat| compat.file_name.to_lowercase());
    Ok(found)
}

/// Jars about to be added to the instance that do not fit its Minecraft version or loader.
/// Unreadable files and files that state nothing are left out, like in [`check_mods`].
pub fn check_new_jars(game_dir: &Path, paths: &[PathBuf], minecraft: &str, loader: Option<Loader>) -> Vec<ModCompat> {
    let Some(loader) = loader else {
        return Vec::new();
    };
    let names = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let installed = std::fs::read_dir(game_dir.join(ResourceKind::Mod.folder()))
        .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase()).collect::<Vec<_>>())
        .unwrap_or_default();
    let has_connector = installed.iter().chain(paths.iter().map(names).collect::<Vec<_>>().iter()).any(|name| name.contains("connector"));
    paths
        .iter()
        .filter(|path| names(path).ends_with(".jar"))
        .filter_map(|path| {
            let descriptors = match std::fs::File::open(path).at(path).and_then(descriptors_of_jar) {
                Ok(descriptors) => descriptors,
                Err(error) => {
                    tracing::warn!("cannot read the descriptors of {}: {error}", path.display());
                    return None;
                }
            };
            let file_name = path.file_name()?.to_string_lossy().into_owned();
            judge(file_name, &descriptors, minecraft, loader, has_connector)
        })
        .collect()
}

fn accepts(instance: Loader, jar: Loader) -> bool {
    match instance {
        Loader::Fabric => jar == Loader::Fabric,
        Loader::Quilt => matches!(jar, Loader::Quilt | Loader::Fabric),
        Loader::Forge => jar == Loader::Forge,
        Loader::NeoForge => matches!(jar, Loader::NeoForge | Loader::Forge),
    }
}

fn judge(file_name: String, descriptors: &[Descriptor], minecraft: &str, loader: Loader, has_connector: bool) -> Option<ModCompat> {
    if descriptors.is_empty() {
        return None;
    }
    let enabled = !file_name.to_ascii_lowercase().ends_with(".disabled");
    let relevant: Vec<&Descriptor> = descriptors.iter().filter(|descriptor| accepts(loader, descriptor.loader)).collect();
    if relevant.is_empty() {
        if matches!(loader, Loader::Forge | Loader::NeoForge) && has_connector {
            return None;
        }
        let mut made_for_loaders: Vec<Loader> = descriptors.iter().map(|descriptor| descriptor.loader).collect();
        made_for_loaders.dedup();
        return Some(ModCompat { file_name, enabled, needs: None, minecraft_mismatch: false, made_for_loaders, loader_mismatch: true });
    }
    let mut wanted: Vec<String> = Vec::new();
    for descriptor in &relevant {
        // no statement, an alternative that matches, or one that cannot be judged: no warning
        let all_wrong = !descriptor.minecraft.is_empty()
            && descriptor.minecraft.iter().all(|spec| matches_spec(descriptor.loader, spec, minecraft) == Some(false));
        if !all_wrong {
            return None;
        }
        wanted.extend(descriptor.minecraft.iter().cloned());
    }
    wanted.dedup();
    Some(ModCompat { file_name, enabled, needs: Some(wanted.join(" | ")), minecraft_mismatch: true, made_for_loaders: Vec::new(), loader_mismatch: false })
}

fn matches_spec(loader: Loader, spec: &str, minecraft: &str) -> Option<bool> {
    let version = minecraft_version(minecraft)?;
    match loader {
        Loader::Fabric | Loader::Quilt => matches_semver(spec, &version),
        // maven ranges know nothing of Minecraft's pre-release naming
        Loader::Forge | Loader::NeoForge => version.pre.is_none().then(|| matches_maven_range(spec, &version)).flatten(),
    }
}

/// Descriptors of a jar, one per recognised descriptor file. A descriptor that cannot be parsed
/// makes the whole jar unknown (an error), because the one that failed could be the one that fits.
pub fn descriptors_of_jar(reader: impl Read + Seek) -> Result<Vec<Descriptor>> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let mut found = Vec::new();
    if let Some(bytes) = entry(&mut archive, "fabric.mod.json")? {
        let json: Value = serde_json::from_slice(&bytes)?;
        found.push(Descriptor { loader: Loader::Fabric, minecraft: json_alternatives(&json["depends"]["minecraft"]) });
    }
    if let Some(bytes) = entry(&mut archive, "quilt.mod.json")? {
        let json: Value = serde_json::from_slice(&bytes)?;
        let minecraft = json["quilt_loader"]["depends"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|dependency| dependency["id"] == "minecraft" && dependency["optional"] != true)
            .flat_map(|dependency| json_alternatives(&dependency["versions"]))
            .collect();
        found.push(Descriptor { loader: Loader::Quilt, minecraft });
    }
    for (path, loader) in [("META-INF/neoforge.mods.toml", Loader::NeoForge), ("META-INF/mods.toml", Loader::Forge)] {
        if let Some(bytes) = entry(&mut archive, path)? {
            found.push(Descriptor { loader, minecraft: mods_toml_ranges(&bytes)? });
        }
    }
    if !found.iter().any(|descriptor| matches!(descriptor.loader, Loader::Forge | Loader::NeoForge))
        && let Some(bytes) = entry(&mut archive, "mcmod.info")?
    {
        let json: Value = serde_json::from_slice(&bytes)?;
        let list = if json.is_array() { &json } else { json.get("modList").or_else(|| json.get("modlist")).unwrap_or(&Value::Null) };
        // "1.12" mods run on 1.12.2, so only the release line has to agree
        let minecraft = list
            .get(0)
            .and_then(|entry| entry["mcversion"].as_str())
            .and_then(Version::parse)
            .filter(|version| version.numbers.len() >= 2)
            .map(|version| vec![format!("{}.{}.x", version.numbers[0], version.numbers[1])])
            .unwrap_or_default();
        found.push(Descriptor { loader: Loader::Forge, minecraft });
    }
    Ok(found)
}

fn entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Result<Option<Vec<u8>>> {
    let mut file = match archive.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).at(name)?;
    Ok(Some(bytes))
}

/// A string or an array of strings, or Quilt's `{any: ..}` of those; anything else (`{all: ..}`) states nothing usable.
fn json_alternatives(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
        Value::Object(map) => map.get("any").map(json_alternatives).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The `versionRange` of every required dependency on `minecraft`.
fn mods_toml_ranges(bytes: &[u8]) -> Result<Vec<String>> {
    let text = std::str::from_utf8(bytes).map_err(|error| Error::Other(format!("mods.toml is not UTF-8: {error}")))?;
    let root: toml::Table = toml::from_str(text).map_err(|error| Error::Other(format!("invalid mods.toml: {error}")))?;
    let mut ranges = Vec::new();
    let tables = root.get("dependencies").and_then(|value| value.as_table()).into_iter().flat_map(|table| table.values());
    for dependency in tables.filter_map(|value| value.as_array()).flatten() {
        let text = |key: &str| dependency.get(key).and_then(|value| value.as_str());
        let optional = dependency.get("mandatory").and_then(|value| value.as_bool()) == Some(false)
            || matches!(text("type"), Some("optional" | "discouraged" | "incompatible"));
        if text("modId") == Some("minecraft")
            && !optional
            && text("side") != Some("SERVER")
            && let Some(range) = text("versionRange")
        {
            ranges.push(range.to_owned());
        }
    }
    Ok(ranges)
}

/// One dot-separated part of a pre-release tag. Numbers sort before words, as in semver.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Number(u64),
    Word(String),
}

/// A version the way Fabric Loader's `SemanticVersion` sees it: numeric components, an optional
/// pre-release tag (`Some(vec![])` is a bare trailing `-`, the lowest pre-release of that version)
/// and ignored `+build` metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    numbers: Vec<u64>,
    pre: Option<Vec<Part>>,
}

impl Version {
    /// `1.21.4`, `1.21.11-`, `1.21-beta.1+build`; `None` for anything else (`24w14a`, `1.x`).
    fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.split_once('+').map_or(text, |(version, _build)| version);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let numbers: Vec<u64> = core
            .split('.')
            .map(|part| if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) { part.parse().ok() } else { None })
            .collect::<Option<_>>()?;
        if !(1..=4).contains(&numbers.len()) {
            return None;
        }
        let pre = match pre {
            None => None,
            Some("") => Some(Vec::new()),
            Some(tag) => Some(
                tag.split('.')
                    .map(|part| match part {
                        "" => None,
                        _ if part.bytes().all(|b| b.is_ascii_digit()) => part.parse().ok().map(Part::Number),
                        _ => Some(Part::Word(part.to_owned())),
                    })
                    .collect::<Option<_>>()?,
            ),
        };
        Some(Version { numbers, pre })
    }

    /// The lowest pre-release of this version cut to its first `len` numbers, the last one raised
    /// by one: the exclusive upper bound of `~`, `^` and `1.x` ranges.
    fn next_after(&self, len: usize) -> Version {
        let mut numbers = self.numbers[..len].to_vec();
        numbers[len - 1] += 1;
        Version { numbers, pre: Some(Vec::new()) }
    }

    /// Compares as if both were padded with zeros (`1.21` equals `1.21.0`); a version with a
    /// pre-release tag is below the same version without one.
    fn compare(&self, other: &Version) -> std::cmp::Ordering {
        (0..self.numbers.len().max(other.numbers.len()))
            .map(|i| self.numbers.get(i).unwrap_or(&0).cmp(other.numbers.get(i).unwrap_or(&0)))
            .find(|ordering| ordering.is_ne())
            .unwrap_or_else(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// The instance's Minecraft version as Fabric Loader normalises it: releases as they are,
/// `1.21-pre1` / `26.1-pre-1` as `beta.1`, `-rc1` as `rc.1`, `26.3-snapshot-1` as `alpha.1`.
/// Weekly snapshots (`24w14a`) cannot be placed without a table of releases and are unknown.
fn minecraft_version(text: &str) -> Option<Version> {
    let text = text.trim();
    let Some((base, tag)) = text.split_once('-') else {
        return Version::parse(text);
    };
    let mut version = Version::parse(base)?;
    let (name, rest) = [("pre", "beta"), ("rc", "rc"), ("snapshot", "alpha")]
        .into_iter()
        .find_map(|(prefix, name)| tag.strip_prefix(prefix).map(|rest| (name, rest)))?;
    let number = rest.trim_start_matches(['-', ' ']);
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    version.pre = Some(vec![Part::Word(name.to_owned()), Part::Number(number.parse().ok()?)]);
    Some(version)
}

/// Fabric/Quilt version predicate: `||` alternatives of space-separated comparators, each `*`,
/// `1.21.x`, or an optional `>=`, `<=`, `>`, `<`, `=`, `^`, `~` before a version. Versions may carry
/// a pre-release tag, and a trailing `-` (`>=1.21.11-`) means the lowest pre-release of that version.
/// `None` when the predicate is not understood.
fn matches_semver(predicate: &str, version: &Version) -> Option<bool> {
    let alternatives = predicate.split("||").map(|alternative| {
        let results: Vec<_> = alternative.split_whitespace().map(|comparator| comparator_matches(comparator, version)).collect();
        if results.is_empty() {
            None
        } else if results.contains(&Some(false)) {
            Some(false)
        } else if results.contains(&None) {
            None
        } else {
            Some(true)
        }
    });
    any_of(alternatives)
}

fn any_of(results: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let results: Vec<_> = results.collect();
    if results.contains(&Some(true)) {
        Some(true)
    } else if results.is_empty() || results.contains(&None) {
        None
    } else {
        Some(false)
    }
}

fn comparator_matches(comparator: &str, version: &Version) -> Option<bool> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    if matches!(comparator, "*" | "x" | "X") {
        return Some(true);
    }
    let operator_len = comparator.bytes().take_while(|b| matches!(b, b'>' | b'<' | b'=' | b'^' | b'~')).count();
    let (operator, rest) = comparator.split_at(operator_len);
    let is_wildcard = |part: &&str| matches!(*part, "x" | "X" | "*");
    if rest.split('.').any(|part| is_wildcard(&part)) {
        // `1.21.x` is [1.21-, 1.22-)
        if !operator.is_empty() && operator != "=" {
            return None;
        }
        let prefix = Version::parse(&rest.split('.').take_while(|part| !is_wildcard(part)).collect::<Vec<_>>().join("."))?;
        let lower = Version { pre: Some(Vec::new()), ..prefix.clone() };
        return Some(version.compare(&lower) != Less && version.compare(&prefix.next_after(prefix.numbers.len())) == Less);
    }
    let wanted = Version::parse(rest)?;
    let ordering = version.compare(&wanted);
    match operator {
        "" | "=" => Some(ordering == Equal),
        ">=" => Some(ordering != Less),
        "<=" => Some(ordering != Greater),
        ">" => Some(ordering == Greater),
        "<" => Some(ordering == Less),
        "^" | "~" => {
            // `~1.2.3` is below 1.3.0 and `~1` below 2.0.0; `^1.2.3` is below 2.0.0
            let upper_len = if operator == "~" && wanted.numbers.len() > 1 { 2 } else { 1 };
            Some(ordering != Less && version.compare(&wanted.next_after(upper_len)) == Less)
        }
        _ => None,
    }
}

/// Maven version range as used by Forge: one or more `[a,b)`-style sets, `[a]` for exactly one
/// version. A bare version is only a recommendation and not judged.
fn matches_maven_range(range: &str, version: &Version) -> Option<bool> {
    let mut results = Vec::new();
    let mut rest = range.trim();
    while !rest.is_empty() {
        let open = rest.chars().next()?;
        if open != '[' && open != '(' {
            return None;
        }
        let end = rest.find([']', ')'])?;
        let close = rest[end..].chars().next()?;
        results.push(set_matches(open == '[', &rest[1..end], close == ']', version));
        rest = rest[end + 1..].trim_start_matches([',', ' ']);
    }
    any_of(results.into_iter())
}

fn set_matches(low_inclusive: bool, inner: &str, high_inclusive: bool, version: &Version) -> Option<bool> {
    let bounds: Vec<&str> = inner.split(',').map(str::trim).collect();
    match bounds.as_slice() {
        [exact] if low_inclusive && high_inclusive => Some(version.compare(&Version::parse(exact)?).is_eq()),
        [low, high] => {
            let low_ok = if low.is_empty() {
                true
            } else {
                let ordering = version.compare(&Version::parse(low)?);
                ordering.is_gt() || (low_inclusive && ordering.is_eq())
            };
            let high_ok = if high.is_empty() {
                true
            } else {
                let ordering = version.compare(&Version::parse(high)?);
                ordering.is_lt() || (high_inclusive && ordering.is_eq())
            };
            Some(low_ok && high_ok)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn semver(predicate: &str, version: &str) -> Option<bool> {
        matches_semver(predicate, &minecraft_version(version).unwrap())
    }

    fn maven(range: &str, version: &str) -> Option<bool> {
        matches_maven_range(range, &Version::parse(version).unwrap())
    }

    #[test]
    fn fabric_comparators() {
        assert_eq!(semver("1.20.1", "1.20.1"), Some(true));
        assert_eq!(semver("1.20.1", "1.20.4"), Some(false));
        assert_eq!(semver("1.21", "1.21.0"), Some(true));
        assert_eq!(semver(">=1.20", "1.21.4"), Some(true));
        assert_eq!(semver(">=1.21.2", "1.21.1"), Some(false));
        assert_eq!(semver(">=1.20 <1.20.5", "1.20.4"), Some(true));
        assert_eq!(semver(">=1.20 <1.20.5", "1.20.5"), Some(false));
        assert_eq!(semver(">1.20", "1.20"), Some(false));
        assert_eq!(semver("<=1.20", "1.20"), Some(true));
        assert_eq!(semver("*", "1.21.4"), Some(true));
    }

    #[test]
    fn fabric_wildcards_tilde_and_caret() {
        assert_eq!(semver("1.21.x", "1.21.4"), Some(true));
        assert_eq!(semver("1.21.x", "1.20.4"), Some(false));
        assert_eq!(semver("1.x", "1.20.4"), Some(true));
        assert_eq!(semver("~1.20", "1.20.6"), Some(true));
        assert_eq!(semver("~1.20", "1.21"), Some(false));
        assert_eq!(semver("~1.20.2", "1.20.1"), Some(false));
        assert_eq!(semver("~1.20.2", "1.20.4"), Some(true));
        assert_eq!(semver("^1.20", "1.21.4"), Some(true));
        assert_eq!(semver("^1.20", "2.0"), Some(false));
    }

    #[test]
    fn fabric_pre_release_predicates() {
        // real descriptors: a trailing `-` is the lowest pre-release of that version
        assert_eq!(semver(">=1.20 <1.20.2-", "1.20.1"), Some(true));
        assert_eq!(semver(">=1.20 <1.20.2-", "1.20.2"), Some(false));
        assert_eq!(semver(">=1.20 <1.20.2-", "1.19.4"), Some(false));
        assert_eq!(semver(">=1.19.4-", "1.20.1"), Some(true));
        assert_eq!(semver(">=1.19.4-", "1.19.3"), Some(false));
        assert_eq!(semver("~1.21.11-", "1.20.1"), Some(false));
        assert_eq!(semver("~1.21.11-", "1.21.11"), Some(true));
        assert_eq!(semver("~1.21.11-", "1.21.12"), Some(true));
        assert_eq!(semver("~1.21.11-", "1.22"), Some(false));
        assert_eq!(semver(">=1.21.11- <1.21.12-", "1.20.1"), Some(false));
        assert_eq!(semver(">=1.21.11- <1.21.12-", "1.21.11"), Some(true));
        assert_eq!(semver(">=1.21.11- <1.21.12-", "1.21.12"), Some(false));
        assert_eq!(semver("^1.21-", "1.21.4"), Some(true));
        assert_eq!(semver("1.21.x", "1.21.2-rc1"), Some(true));
        // tags and build metadata
        assert_eq!(semver(">=1.21-beta.1", "1.21"), Some(true));
        assert_eq!(semver(">=1.21-beta.1", "1.20.4"), Some(false));
        assert_eq!(semver(">=1.21+build.5", "1.21"), Some(true));
        assert_eq!(semver(">=1.20.5-alpha.24.14.a", "1.20.4"), Some(false));
        assert_eq!(semver(">=1.20.5-alpha.24.14.a", "1.20.5"), Some(true));
        assert_eq!(semver("<1.21", "1.21-rc1"), Some(true));
        assert_eq!(semver(">=1.21", "1.21-rc1"), Some(false));
        assert_eq!(semver(">=1.21-", "1.21-rc1"), Some(true));
        assert_eq!(semver(">=1.21-beta.1", "1.21-pre1"), Some(true));
        assert_eq!(semver(">=1.21-beta.2", "1.21-pre1"), Some(false));
        assert_eq!(semver("<1.21-rc.1", "1.21-pre2"), Some(true));
        // `X` and `*` wildcards
        assert_eq!(semver("1.21.X", "1.21.4"), Some(true));
        assert_eq!(semver("1.21.*", "1.20.4"), Some(false));
    }

    #[test]
    fn new_style_minecraft_versions() {
        assert_eq!(semver(">=26.1", "26.2"), Some(true));
        assert_eq!(semver(">=26.1", "1.21.11"), Some(false));
        assert_eq!(semver("~1.21.11-", "26.1"), Some(false));
        assert_eq!(semver(">=26.1- <26.2-", "26.1"), Some(true));
        assert_eq!(semver(">=26.1- <26.2-", "26.2"), Some(false));
        assert_eq!(semver(">=26.1 <26.3", "26.3-snapshot-1"), Some(true));
        assert_eq!(semver(">=26.3", "26.3-snapshot-1"), Some(false));
        assert_eq!(semver(">=26.3-", "26.3-snapshot-1"), Some(true));
        assert_eq!(semver("26.1.x", "26.1.2"), Some(true));
        assert_eq!(maven("[26.1]", "26.1"), Some(true));
        assert_eq!(maven("[26.1,26.2)", "26.2"), Some(false));
        assert_eq!(matches_spec(Loader::Forge, "[26.1,26.2)", "26.3-snapshot-1"), None);
        assert_eq!(matches_spec(Loader::Fabric, ">=26.3", "26.3-snapshot-1"), Some(false));
        assert_eq!(matches_spec(Loader::Fabric, ">=1.21", "1.21.11-rc1"), Some(true));
        assert_eq!(matches_spec(Loader::Fabric, ">=1.21", "1.21.11-rc-1"), Some(true));
    }

    #[test]
    fn fabric_api_for_1_21_11_does_not_fit_1_20_1() {
        for predicate in ["~1.21.11-", ">=1.21.11- <1.21.12-"] {
            let text = format!(r#"{{"id":"fabric-api","version":"0.141.4+1.21.11","depends":{{"fabricloader":">=0.18.1","minecraft":"{predicate}"}}}}"#);
            let descriptors = descriptors_of_jar(jar(&[("fabric.mod.json", &text)])).unwrap();
            let found = judge("fabric-api-0.141.4+1.21.11.jar".into(), &descriptors, "1.20.1", Loader::Fabric, false).unwrap();
            assert!(found.minecraft_mismatch && found.needs.as_deref() == Some(predicate));
            assert!(judge("fabric-api.jar".into(), &descriptors, "1.21.11", Loader::Fabric, false).is_none());
        }
    }

    #[test]
    fn unreadable_jars_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let (game, instance) = (dir.path().join("game"), dir.path().join("inst"));
        std::fs::create_dir_all(game.join("mods")).unwrap();
        std::fs::create_dir_all(&instance).unwrap();
        std::fs::write(game.join("mods").join("half.jar"), b"not a zip").unwrap();
        std::fs::write(game.join("mods").join("good.jar"), jar(&[("fabric.mod.json", r#"{"id":"g"}"#)]).into_inner()).unwrap();
        check_mods(&game, &instance, "1.21.4", Some(Loader::Fabric)).unwrap();
        let cache: BTreeMap<String, Vec<Descriptor>> = serde_json::from_slice(&std::fs::read(instance.join(CACHE_FILE)).unwrap()).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.keys().all(|key| key.starts_with("good.jar|")));
    }

    #[test]
    fn fabric_alternatives() {
        assert_eq!(semver("1.20.1 || 1.20.4", "1.20.4"), Some(true));
        assert_eq!(semver("1.20.1 || 1.20.4", "1.21"), Some(false));
        // an alternative that cannot be judged keeps the verdict open
        assert_eq!(semver("1.20.1 || 24w14a", "1.21"), None);
        assert_eq!(semver("1.20.1 || 24w14a", "1.20.1"), Some(true));
    }

    #[test]
    fn unparsable_input_is_unknown() {
        assert_eq!(semver("", "1.21"), None);
        assert_eq!(semver("garbage", "1.21"), None);
        assert_eq!(semver(">=24w14a", "1.21"), None);
        assert_eq!(Version::parse("24w14a"), None);
        assert_eq!(minecraft_version("24w14a"), None);
        assert_eq!(minecraft_version("1.21-pre"), None);
        assert_eq!(matches_spec(Loader::Fabric, "1.20.1", "24w14a"), None);
    }

    #[test]
    fn maven_ranges() {
        assert_eq!(maven("[1.20.1,1.21)", "1.20.4"), Some(true));
        assert_eq!(maven("[1.20.1,1.21)", "1.21"), Some(false));
        assert_eq!(maven("[1.20.1,1.21)", "1.20"), Some(false));
        assert_eq!(maven("[1.20.1]", "1.20.1"), Some(true));
        assert_eq!(maven("[1.20.1]", "1.20.2"), Some(false));
        assert_eq!(maven("[1.21,)", "1.21.4"), Some(true));
        assert_eq!(maven("(,1.20.1]", "1.20.1"), Some(true));
        assert_eq!(maven("(,1.20.1]", "1.21"), Some(false));
        assert_eq!(maven("(1.20,1.21]", "1.20"), Some(false));
        assert_eq!(maven("[1.19,1.20),[1.21,)", "1.21.1"), Some(true));
        assert_eq!(maven("[1.19,1.20), [1.21,)", "1.20.4"), Some(false));
    }

    #[test]
    fn maven_unknowns() {
        assert_eq!(maven("1.20.1", "1.20.1"), None);
        assert_eq!(maven("", "1.20.1"), None);
        assert_eq!(maven("[1.20.1", "1.20.1"), None);
        assert_eq!(maven("[${mcversion}]", "1.20.1"), None);
    }

    fn jar(entries: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in entries {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        let mut cursor = writer.finish().unwrap();
        cursor.set_position(0);
        cursor
    }

    #[test]
    fn descriptors_of_each_loader() {
        let fabric = descriptors_of_jar(jar(&[("fabric.mod.json", r#"{"id":"a","depends":{"minecraft":["1.20.1","1.20.4"]}}"#)])).unwrap();
        assert_eq!(fabric, [Descriptor { loader: Loader::Fabric, minecraft: vec!["1.20.1".into(), "1.20.4".into()] }]);
        let quilt = descriptors_of_jar(jar(&[("quilt.mod.json", r#"{"quilt_loader":{"depends":[{"id":"minecraft","versions":">=1.20"},"other"]}}"#)])).unwrap();
        assert_eq!(quilt[0].minecraft, [">=1.20"]);
        let quilt_any = descriptors_of_jar(jar(&[("quilt.mod.json", r#"{"quilt_loader":{"depends":[{"id":"minecraft","versions":{"any":["1.20.1","~1.21.11-"]}}]}}"#)])).unwrap();
        assert_eq!(quilt_any[0].minecraft, ["1.20.1", "~1.21.11-"]);
        let toml = "[[mods]]\nmodId=\"a\"\n[[dependencies.a]]\nmodId=\"forge\"\nversionRange=\"[47,)\"\n[[dependencies.a]]\nmodId=\"minecraft\"\nmandatory=true\nversionRange=\"[1.20.1,1.21)\"\n";
        let forge = descriptors_of_jar(jar(&[("META-INF/mods.toml", toml)])).unwrap();
        assert_eq!(forge, [Descriptor { loader: Loader::Forge, minecraft: vec!["[1.20.1,1.21)".into()] }]);
        let optional = toml.replace("mandatory=true", "mandatory=false");
        assert!(descriptors_of_jar(jar(&[("META-INF/mods.toml", &optional)])).unwrap()[0].minecraft.is_empty());
        let legacy = descriptors_of_jar(jar(&[("mcmod.info", r#"[{"modid":"a","name":"A","mcversion":"1.12.2"}]"#)])).unwrap();
        assert_eq!(legacy, [Descriptor { loader: Loader::Forge, minecraft: vec!["1.12.x".into()] }]);
        let template = descriptors_of_jar(jar(&[("mcmod.info", r#"[{"modid":"a","name":"A","mcversion":"${mcversion}"}]"#)])).unwrap();
        assert!(template[0].minecraft.is_empty());
        assert!(descriptors_of_jar(jar(&[("fabric.mod.json", "{broken")])).is_err());
        assert!(descriptors_of_jar(jar(&[("readme.txt", "x")])).unwrap().is_empty());
    }

    #[test]
    fn a_turned_off_loader_is_no_loader() {
        let component = |uid: &str, disabled| ComponentSpec { uid: uid.into(), version: "1".into(), dependency_only: false, disabled };
        assert_eq!(loader_of_components(&[component("net.fabricmc.fabric-loader", false)]), Some(Loader::Fabric));
        assert_eq!(loader_of_components(&[component("net.fabricmc.fabric-loader", true)]), None);
    }

    fn fabric(spec: &[&str]) -> Descriptor {
        Descriptor { loader: Loader::Fabric, minecraft: spec.iter().map(|s| (*s).to_owned()).collect() }
    }

    #[test]
    fn judging_version_and_loader() {
        let verdict = |descriptors: &[Descriptor], loader| judge("m.jar".into(), descriptors, "1.21.4", loader, false);
        let wrong = verdict(&[fabric(&["1.20.1"])], Loader::Fabric).unwrap();
        assert!(wrong.minecraft_mismatch && wrong.needs.as_deref() == Some("1.20.1") && wrong.enabled);
        assert!(verdict(&[fabric(&["1.21.x"])], Loader::Fabric).is_none());
        assert!(verdict(&[fabric(&[])], Loader::Fabric).is_none());
        assert!(verdict(&[fabric(&["1.20.1", "garbage"])], Loader::Fabric).is_none());
        // Quilt runs Fabric mods
        assert!(verdict(&[fabric(&["1.21.4"])], Loader::Quilt).is_none());
        let forge_only = [Descriptor { loader: Loader::Forge, minecraft: vec!["[1.20.1]".into()] }];
        let loader = verdict(&forge_only, Loader::Fabric).unwrap();
        assert!(loader.loader_mismatch && !loader.minecraft_mismatch && loader.made_for_loaders == [Loader::Forge]);
        // a jar for several loaders is fine when one fits, judged by that one
        let both = [forge_only[0].clone(), fabric(&["1.21.4"])];
        assert!(verdict(&both, Loader::Fabric).is_none());
        assert!(judge("m.jar".into(), &[fabric(&[])], "1.21.4", Loader::Forge, true).is_none());
        assert!(judge("m.jar".into(), &[fabric(&[])], "1.21.4", Loader::Forge, false).unwrap().loader_mismatch);
        assert!(!judge("m.jar.disabled".into(), &[fabric(&["1.20.1"])], "1.21.4", Loader::Fabric, false).unwrap().enabled);
        assert!(verdict(&[], Loader::Fabric).is_none());
    }

    #[test]
    fn check_mods_reads_caches_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let (game, instance) = (dir.path().join("game"), dir.path().join("inst"));
        std::fs::create_dir_all(game.join("mods")).unwrap();
        std::fs::create_dir_all(&instance).unwrap();
        let write = |name: &str, text: &str| std::fs::write(game.join("mods").join(name), jar(&[("fabric.mod.json", text)]).into_inner()).unwrap();
        write("old.jar", r#"{"id":"o","depends":{"minecraft":"~1.20.1"}}"#);
        write("ok.jar", r#"{"id":"k","depends":{"minecraft":">=1.21"}}"#);
        std::fs::write(game.join("mods").join("lib.jar"), b"not a zip").unwrap();
        let found = check_mods(&game, &instance, "1.21.4", Some(Loader::Fabric)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].file_name.as_str(), found[0].needs.as_deref()), ("old.jar", Some("~1.20.1")));
        assert!(instance.join(CACHE_FILE).exists());
        // the second run is answered from the cache
        assert_eq!(check_mods(&game, &instance, "1.21.4", Some(Loader::Fabric)).unwrap(), found);
        assert!(check_mods(&game, &instance, "1.21.4", None).unwrap().is_empty());
    }

    #[test]
    fn new_jars_are_judged_before_they_are_added() {
        let dir = tempfile::tempdir().unwrap();
        let (game, incoming) = (dir.path().join("game"), dir.path().join("incoming"));
        std::fs::create_dir_all(&incoming).unwrap();
        let write = |name: &str, text: &str| {
            let path = incoming.join(name);
            std::fs::write(&path, jar(&[("fabric.mod.json", text)]).into_inner()).unwrap();
            path
        };
        let old = write("old.jar", r#"{"id":"o","depends":{"minecraft":"1.20.1"}}"#);
        let ok = write("ok.jar", r#"{"id":"k","depends":{"minecraft":"1.21.x"}}"#);
        let broken = incoming.join("broken.jar");
        std::fs::write(&broken, b"not a zip").unwrap();
        let pack = incoming.join("pack.zip");
        std::fs::write(&pack, b"").unwrap();
        // the game folder may not have a mods folder yet
        let found = check_new_jars(&game, &[old, ok, broken, pack], "1.21.4", Some(Loader::Fabric));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].file_name.as_str(), found[0].needs.as_deref()), ("old.jar", Some("1.20.1")));
        assert!(check_new_jars(&game, &[incoming.join("old.jar")], "1.21.4", None).is_empty());
    }
}
