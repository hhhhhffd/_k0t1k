//! Does an installed mod jar declare itself for the instance's Minecraft version and loader?
//! Only what the jar's own descriptor states is checked; anything unreadable or not understood is
//! "unknown" and never reported, so a warning always has a stated reason behind it.
//! All functions here do blocking file I/O; call them from `spawn_blocking` in async code.

use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::path::Path;

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
                    tracing::warn!("cannot read the descriptors of {}: {error}", path.display());
                    Vec::new()
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
    let version = parse_version(minecraft)?;
    match loader {
        Loader::Fabric | Loader::Quilt => matches_semver(spec, &version),
        Loader::Forge | Loader::NeoForge => matches_maven_range(spec, &version),
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
            .and_then(parse_version)
            .filter(|version| version.len() >= 2)
            .map(|version| vec![format!("{}.{}.x", version[0], version[1])])
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

/// A string or an array of strings; anything else (Quilt's `{all: ..}` objects) states nothing usable.
fn json_alternatives(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
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

/// `1.21.4` as numbers; `None` for snapshots, pre-releases and anything else not purely numeric.
fn parse_version(text: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = text
        .trim()
        .split('.')
        .map(|part| if part.bytes().all(|b| b.is_ascii_digit()) { part.parse().ok() } else { None })
        .collect::<Option<_>>()?;
    (1..=4).contains(&parts.len()).then_some(parts)
}

/// Compares as if both were padded with zeros (`1.21` equals `1.21.0`).
fn compare(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|ordering| ordering.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Fabric/Quilt version predicate: `||` alternatives of space-separated comparators, each `*`,
/// `1.21.x`, or an optional `>=`, `<=`, `>`, `<`, `=`, `^`, `~` before a version.
/// `None` when the predicate is not understood.
fn matches_semver(predicate: &str, version: &[u64]) -> Option<bool> {
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

fn comparator_matches(comparator: &str, version: &[u64]) -> Option<bool> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    if matches!(comparator, "*" | "x" | "X") {
        return Some(true);
    }
    let operator_len = comparator.bytes().take_while(|b| matches!(b, b'>' | b'<' | b'=' | b'^' | b'~')).count();
    let (operator, rest) = comparator.split_at(operator_len);
    let is_wildcard = |part: &&str| matches!(*part, "x" | "X" | "*");
    if rest.split('.').any(|part| is_wildcard(&part)) {
        // `1.21.x`: every part before the wildcard must be equal
        if !operator.is_empty() && operator != "=" {
            return None;
        }
        let prefix = parse_version(&rest.split('.').take_while(|part| !is_wildcard(part)).collect::<Vec<_>>().join("."))?;
        return Some(prefix.iter().enumerate().all(|(i, part)| version.get(i).unwrap_or(&0) == part));
    }
    let wanted = parse_version(rest)?;
    let ordering = compare(version, &wanted);
    match operator {
        "" | "=" => Some(ordering == Equal),
        ">=" => Some(ordering != Less),
        "<=" => Some(ordering != Greater),
        ">" => Some(ordering == Greater),
        "<" => Some(ordering == Less),
        "^" | "~" => {
            // `~1.2.3` is below 1.3.0 and `~1` below 2.0.0; `^1.2.3` is below 2.0.0 (not defined for 0.x)
            let upper = if operator == "^" {
                if wanted[0] == 0 {
                    return None;
                }
                vec![wanted[0] + 1]
            } else if wanted.len() == 1 {
                vec![wanted[0] + 1]
            } else {
                vec![wanted[0], wanted[1] + 1]
            };
            Some(ordering != Less && compare(version, &upper) == Less)
        }
        _ => None,
    }
}

/// Maven version range as used by Forge: one or more `[a,b)`-style sets, `[a]` for exactly one
/// version. A bare version is only a recommendation and not judged.
fn matches_maven_range(range: &str, version: &[u64]) -> Option<bool> {
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

fn set_matches(low_inclusive: bool, inner: &str, high_inclusive: bool, version: &[u64]) -> Option<bool> {
    let bounds: Vec<&str> = inner.split(',').map(str::trim).collect();
    match bounds.as_slice() {
        [exact] if low_inclusive && high_inclusive => Some(compare(version, &parse_version(exact)?).is_eq()),
        [low, high] => {
            let low_ok = if low.is_empty() {
                true
            } else {
                let ordering = compare(version, &parse_version(low)?);
                ordering.is_gt() || (low_inclusive && ordering.is_eq())
            };
            let high_ok = if high.is_empty() {
                true
            } else {
                let ordering = compare(version, &parse_version(high)?);
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
        matches_semver(predicate, &parse_version(version).unwrap())
    }

    fn maven(range: &str, version: &str) -> Option<bool> {
        matches_maven_range(range, &parse_version(version).unwrap())
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
        assert_eq!(semver(">=1.21-beta.1", "1.21"), None);
        assert_eq!(semver("garbage", "1.21"), None);
        assert_eq!(parse_version("24w14a"), None);
        assert_eq!(parse_version("1.21-pre1"), None);
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
}
