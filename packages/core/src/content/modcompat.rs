//! Do the installed mod jars fit the instance? Four signals, each with a stated reason:
//! 1. what the jar's own descriptor says about Minecraft and the loader;
//! 2. required dependencies (and their versions) that no other enabled jar provides;
//! 3. for jars whose descriptor is silent about Minecraft: the game versions Modrinth lists for the file's hash;
//! 4. as a last resort, an explicit `+mc1.21.11` style version in the file name.
//!
//! Anything unreadable or not understood is "unknown" and never reported.
//! Descriptors and Modrinth answers are cached in `<instance>/modcompat.json` by file name, size and
//! modification time. The checks do their blocking file I/O on blocking threads.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Loader, ResourceKind, modrinth};
use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::error::{Error, IoContext, Result};
use crate::scancache;

const CACHE_FILE: &str = "modcompat.json";
/// Bumped when the cached shapes change; a cache of another version is dropped and rebuilt.
const CACHE_VERSION: u32 = 2;
/// Jars inside jars are read this deep (Fabric API nests its modules one level down).
const MAX_NESTED_DEPTH: u8 = 3;
/// Ids the game or the loader provides itself; a dependency on one of them is never "missing".
/// `mixinextras` is bundled into recent Fabric and Quilt loaders.
const PLATFORM_IDS: [&str; 13] = [
    "minecraft",
    "java",
    "fabricloader",
    "fabric-loader",
    "quilt_loader",
    "quilt_base",
    "forge",
    "neoforge",
    "fml",
    "javafml",
    "lowcodefml",
    "mclanguage",
    "mixinextras",
];

/// A mod id a jar makes available, with its version when the descriptor states one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provided {
    pub id: String,
    pub version: Option<String>,
}

/// A required dependency. `versions` holds alternatives (any may match); empty means any version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub versions: Vec<String>,
}

/// What one descriptor file of a jar says. `minecraft` holds alternatives (any may match); it is
/// empty when the descriptor states nothing about Minecraft. Fabric and Quilt use semver
/// predicates, Forge and NeoForge maven ranges, legacy `mcmod.info` becomes a `1.12.x` wildcard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub loader: Loader,
    pub minecraft: Vec<String>,
    /// Own ids, `provides`, and the ids of nested jars.
    pub provides: Vec<Provided>,
    pub requires: Vec<Requirement>,
}

/// What Modrinth knows about a file, found by its SHA-1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthInfo {
    pub game_versions: Vec<String>,
}

/// Who says the Minecraft version does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MismatchSource {
    Descriptor,
    Modrinth,
    /// A guess from the file name; the weakest signal.
    Filename,
}

/// A required dependency that is absent (`installed` is `None`) or has the wrong version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingDependency {
    pub id: String,
    pub wanted: Option<String>,
    pub installed: Option<String>,
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
    /// Where `needs` comes from; set only for a version mismatch.
    pub source: Option<MismatchSource>,
    /// Loaders the jar is made for; set only for a loader mismatch.
    pub made_for_loaders: Vec<Loader>,
    pub loader_mismatch: bool,
    /// Required mods that no enabled jar of the instance provides in a fitting version.
    pub dependencies: Vec<MissingDependency>,
}

#[derive(Default, Clone, PartialEq, Serialize, Deserialize)]
struct Cache {
    version: u32,
    descriptors: BTreeMap<String, Vec<Descriptor>>,
    /// `None`: Modrinth does not know the file. A failed request is never stored.
    modrinth: BTreeMap<String, Option<ModrinthInfo>>,
}

impl Cache {
    /// A missing, unreadable or outdated cache is empty: jars are simply read again.
    fn load(path: &Path) -> Cache {
        let cache = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok());
        cache.filter(|cache| cache.version == CACHE_VERSION).unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        match serde_json::to_vec(self) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(path, bytes) {
                    tracing::warn!("cannot write {}: {error}", path.display());
                }
            }
            Err(error) => tracing::warn!("cannot serialize the mod compatibility cache: {error}"),
        }
    }
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

struct Jar {
    file_name: String,
    path: PathBuf,
    key: String,
    descriptors: Vec<Descriptor>,
    /// False when the jar could not be opened (locked, half-copied): it is not cached.
    readable: bool,
    /// Whether the jar is judged, as opposed to only providing mods to the others.
    target: bool,
}

impl Jar {
    fn enabled(&self) -> bool {
        !self.file_name.to_ascii_lowercase().ends_with(".disabled")
    }
}

/// Mod jars of the instance that do not fit its Minecraft version or loader, miss a dependency, or
/// that Modrinth lists for other game versions. Needs the network only for jars whose descriptor
/// does not name a Minecraft version, and only once per jar; a failed lookup just gives no verdict.
pub async fn check_mods(modrinth: &modrinth::Client, game_dir: &Path, instance_dir: &Path, minecraft: &str, loader: Option<Loader>) -> Result<Vec<ModCompat>> {
    check(|hashes| lookup_modrinth(modrinth, hashes), game_dir, instance_dir, minecraft, loader, None).await
}

/// Jars about to be added to the instance, judged together with the jars already installed.
/// Unreadable files and files that state nothing are left out, like in [`check_mods`].
pub async fn check_new_jars(
    modrinth: &modrinth::Client,
    game_dir: &Path,
    instance_dir: &Path,
    paths: Vec<PathBuf>,
    minecraft: &str,
    loader: Option<Loader>,
) -> Result<Vec<ModCompat>> {
    check(|hashes| lookup_modrinth(modrinth, hashes), game_dir, instance_dir, minecraft, loader, Some(paths)).await
}

/// One request for all hashes. The answer is keyed by lowercase hash; hashes Modrinth does not know are absent.
async fn lookup_modrinth(client: &modrinth::Client, hashes: Vec<String>) -> Result<HashMap<String, ModrinthInfo>> {
    let found = client.versions_by_hashes(&hashes, modrinth::HashAlgorithm::Sha1).await?;
    Ok(found.into_iter().map(|(hash, version)| (hash.to_ascii_lowercase(), ModrinthInfo { game_versions: version.game_versions })).collect())
}

struct Prepared {
    jars: Vec<Jar>,
    old: Cache,
    /// Cache key and SHA-1 of the jars to ask Modrinth about.
    pending: Vec<(String, String)>,
}

/// `incoming`: judge only these files, with the installed jars as the context; `None`: judge the installed jars.
async fn check(
    lookup: impl AsyncFnOnce(Vec<String>) -> Result<HashMap<String, ModrinthInfo>>,
    game_dir: &Path,
    instance_dir: &Path,
    minecraft: &str,
    loader: Option<Loader>,
    incoming: Option<Vec<PathBuf>>,
) -> Result<Vec<ModCompat>> {
    let Some(loader) = loader else {
        return Ok(Vec::new());
    };
    let (game_dir, cache_path, minecraft) = (game_dir.to_owned(), instance_dir.join(CACHE_FILE), minecraft.to_owned());
    let Prepared { jars, old, pending } = {
        let cache_path = cache_path.clone();
        tokio::task::spawn_blocking(move || prepare(&game_dir, &cache_path, loader, incoming)).await.map_err(|error| Error::Other(error.to_string()))??
    };

    let mut fetched: BTreeMap<String, Option<ModrinthInfo>> = BTreeMap::new();
    if !pending.is_empty() {
        let hashes = pending.iter().map(|(_, hash)| hash.clone()).collect();
        match lookup(hashes).await {
            Ok(found) => fetched.extend(pending.iter().map(|(key, hash)| (key.clone(), found.get(hash).cloned()))),
            Err(error) => tracing::warn!("Modrinth lookup of the mod files failed: {error}"),
        }
    }

    tokio::task::spawn_blocking(move || {
        let mut cache = Cache { version: CACHE_VERSION, ..Cache::default() };
        for jar in &jars {
            if jar.readable {
                cache.descriptors.insert(jar.key.clone(), jar.descriptors.clone());
            }
            if let Some(known) = fetched.get(&jar.key).or_else(|| old.modrinth.get(&jar.key)) {
                cache.modrinth.insert(jar.key.clone(), known.clone());
            }
        }
        if cache != old {
            cache.save(&cache_path);
        }
        judge_all(&jars, &cache, &minecraft, loader)
    })
    .await
    .map_err(|error| Error::Other(error.to_string()))
}

fn prepare(game_dir: &Path, cache_path: &Path, loader: Loader, incoming: Option<Vec<PathBuf>>) -> Result<Prepared> {
    let old = Cache::load(cache_path);
    let folder = game_dir.join(ResourceKind::Mod.folder());
    let mut paths = match std::fs::read_dir(&folder) {
        Ok(entries) => entries.map(|entry| entry.map(|entry| entry.path()).at(&folder)).collect::<Result<Vec<_>>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).at(&folder),
    };
    let name_of = |path: &Path| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let targets: Option<HashSet<String>> = incoming.as_ref().map(|files| files.iter().map(|path| name_of(path)).collect());
    if let (Some(files), Some(names)) = (incoming, &targets) {
        // an incoming file replaces an installed one of the same name
        paths.retain(|path| !names.contains(&name_of(path)));
        paths.extend(files);
    }
    let mut jars: Vec<Jar> = paths.iter().filter_map(|path| read_jar(path, &old)).collect();
    for jar in &mut jars {
        jar.target = targets.as_ref().is_none_or(|names| names.contains(&jar.file_name));
    }
    let mut pending = Vec::new();
    for jar in jars.iter().filter(|jar| jar.target && jar.readable && !old.modrinth.contains_key(&jar.key)) {
        if !needs_modrinth(&jar.descriptors, loader) {
            continue;
        }
        match scancache::sha1_of(&jar.path) {
            Ok(hash) => pending.push((jar.key.clone(), hash)),
            Err(error) => tracing::warn!("cannot hash {}: {error}", jar.path.display()),
        }
    }
    Ok(Prepared { jars, old, pending })
}

/// `None` for anything that is not a mod jar. Descriptors come from the cache when file name, size and time agree.
fn read_jar(path: &Path, cache: &Cache) -> Option<Jar> {
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let lower = file_name.to_ascii_lowercase();
    if !(lower.ends_with(".jar") || lower.ends_with(".jar.disabled")) {
        return None;
    }
    let meta = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return None,
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", path.display());
            return None;
        }
    };
    let modified = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let key = format!("{file_name}|{}|{modified}", meta.len());
    let (descriptors, readable) = match cache.descriptors.get(&key) {
        Some(cached) => (cached.clone(), true),
        None => match std::fs::File::open(path).at(path).and_then(descriptors_of_jar) {
            Ok(descriptors) => (descriptors, true),
            Err(error) => {
                // not cached, so a jar that was locked or half-copied is read again next time
                tracing::warn!("cannot read the descriptors of {}: {error}", path.display());
                (Vec::new(), false)
            }
        },
    };
    Some(Jar { file_name, path: path.to_owned(), key, descriptors, readable, target: true })
}

fn accepts(instance: Loader, jar: Loader) -> bool {
    match instance {
        Loader::Fabric => jar == Loader::Fabric,
        Loader::Quilt => matches!(jar, Loader::Quilt | Loader::Fabric),
        Loader::Forge => jar == Loader::Forge,
        Loader::NeoForge => matches!(jar, Loader::NeoForge | Loader::Forge),
    }
}

fn relevant(descriptors: &[Descriptor], loader: Loader) -> Vec<&Descriptor> {
    descriptors.iter().filter(|descriptor| accepts(loader, descriptor.loader)).collect()
}

/// Modrinth is asked only when the descriptors that matter say nothing about Minecraft.
fn needs_modrinth(descriptors: &[Descriptor], loader: Loader) -> bool {
    let relevant = relevant(descriptors, loader);
    !relevant.is_empty() && relevant.iter().all(|descriptor| descriptor.minecraft.is_empty())
}

struct Context {
    minecraft: String,
    loader: Loader,
    /// A Fabric jar can run on Forge through Sinytra Connector; with it around, loaders and dependencies are not judged.
    has_connector: bool,
    /// Lowercase mod id to the versions of every enabled jar that provides it.
    providers: HashMap<String, Vec<Option<String>>>,
}

fn judge_all(jars: &[Jar], cache: &Cache, minecraft: &str, loader: Loader) -> Vec<ModCompat> {
    let mut providers: HashMap<String, Vec<Option<String>>> = HashMap::new();
    for jar in jars.iter().filter(|jar| jar.enabled()) {
        for descriptor in relevant(&jar.descriptors, loader) {
            for provided in &descriptor.provides {
                providers.entry(provided.id.to_ascii_lowercase()).or_default().push(provided.version.clone());
            }
        }
    }
    let has_connector = jars.iter().any(|jar| jar.file_name.to_ascii_lowercase().contains("connector"));
    let context = Context { minecraft: minecraft.to_owned(), loader, has_connector, providers };
    let mut found: Vec<ModCompat> = jars
        .iter()
        .filter(|jar| jar.target)
        .filter_map(|jar| judge(jar.file_name.clone(), &jar.descriptors, &context, cache.modrinth.get(&jar.key).and_then(Option::as_ref)))
        .collect();
    found.sort_by_key(|compat| compat.file_name.to_lowercase());
    found
}

fn judge(file_name: String, descriptors: &[Descriptor], context: &Context, modrinth: Option<&ModrinthInfo>) -> Option<ModCompat> {
    if descriptors.is_empty() {
        return None;
    }
    let enabled = !file_name.to_ascii_lowercase().ends_with(".disabled");
    let relevant = relevant(descriptors, context.loader);
    if relevant.is_empty() {
        if matches!(context.loader, Loader::Forge | Loader::NeoForge) && context.has_connector {
            return None;
        }
        let mut made_for_loaders: Vec<Loader> = descriptors.iter().map(|descriptor| descriptor.loader).collect();
        made_for_loaders.dedup();
        return Some(ModCompat { file_name, enabled, needs: None, minecraft_mismatch: false, source: None, made_for_loaders, loader_mismatch: true, dependencies: Vec::new() });
    }
    let version_problem = version_problem(&file_name, &relevant, context, modrinth);
    // a disabled jar does not load, so what it needs does not matter yet
    let skip_dependencies = !enabled || (context.has_connector && matches!(context.loader, Loader::Forge | Loader::NeoForge));
    let dependencies = if skip_dependencies { Vec::new() } else { missing_dependencies(&relevant, context) };
    if version_problem.is_none() && dependencies.is_empty() {
        return None;
    }
    let (needs, source) = version_problem.unzip();
    Some(ModCompat { file_name, enabled, minecraft_mismatch: needs.is_some(), needs, source, made_for_loaders: Vec::new(), loader_mismatch: false, dependencies })
}

/// The Minecraft versions the jar is made for, when they do not include the instance's, and who says so.
fn version_problem(file_name: &str, relevant: &[&Descriptor], context: &Context, modrinth: Option<&ModrinthInfo>) -> Option<(String, MismatchSource)> {
    let minecraft = context.minecraft.as_str();
    // no statement, an alternative that matches, or one that cannot be judged: no warning
    let all_wrong = |descriptor: &&Descriptor| {
        !descriptor.minecraft.is_empty() && descriptor.minecraft.iter().all(|spec| matches_spec(descriptor.loader, spec, minecraft) == Some(false))
    };
    if relevant.iter().all(all_wrong) {
        let mut wanted: Vec<String> = relevant.iter().flat_map(|descriptor| descriptor.minecraft.iter().cloned()).collect();
        wanted.dedup();
        return Some((wanted.join(" | "), MismatchSource::Descriptor));
    }
    if relevant.iter().any(|descriptor| !descriptor.minecraft.is_empty()) {
        return None;
    }
    // the descriptor is silent; a file Modrinth knows is judged by its data alone
    if let Some(info) = modrinth {
        let listed = &info.game_versions;
        if listed.is_empty() || listed.iter().any(|version| same_version(version, minecraft)) {
            return None;
        }
        let shown = if listed.len() > 4 { format!("{}, ...", listed[..3].join(", ")) } else { listed.join(", ") };
        return Some((shown, MismatchSource::Modrinth));
    }
    filename_minecraft(file_name, minecraft).map(|versions| (versions, MismatchSource::Filename))
}

fn same_version(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
        || match (minecraft_version(a), minecraft_version(b)) {
            (Some(a), Some(b)) => a.compare(&b).is_eq(),
            _ => false,
        }
}

/// Explicit Minecraft versions in a file name (`+mc1.21.11`, `-mc1.20.1`, `+1.20.1`), when every one of them
/// is on another release line than the instance (1.21.x against 1.20.1). Mods are often built against one
/// patch version and run on its neighbours, so the same line is never reported. A bare `1.21.11` is not
/// taken for a Minecraft version: it is usually the mod's own.
fn filename_minecraft(file_name: &str, minecraft: &str) -> Option<String> {
    let instance = minecraft_version(minecraft).filter(|version| version.pre.is_none() && version.numbers.len() >= 2)?;
    let name = file_name.to_ascii_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    for (index, character) in name.char_indices() {
        let start = if character == '+' {
            index + 1
        } else if name[index..].starts_with("mc") && !name[..index].chars().next_back().is_some_and(|before| before.is_ascii_alphanumeric()) {
            index + 2
        } else {
            continue;
        };
        let token: String = name[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let token = token.trim_end_matches('.');
        let numbers: Vec<u64> = token.split('.').filter_map(|part| part.parse().ok()).collect();
        let is_release = numbers.len() == token.split('.').count()
            && (2..=3).contains(&numbers.len())
            && ((numbers[0] == 1 && (7..=21).contains(&numbers[1])) || numbers[0] >= 26);
        if is_release {
            if numbers[..2] == instance.numbers[..2] {
                return None;
            }
            if !tokens.iter().any(|known| known == token) {
                tokens.push(token.to_owned());
            }
        }
    }
    (!tokens.is_empty()).then(|| tokens.join(", "))
}

/// Required dependencies of the descriptor the loader would use that no enabled jar provides, or provides
/// only in versions that do not satisfy the requirement.
fn missing_dependencies(relevant: &[&Descriptor], context: &Context) -> Vec<MissingDependency> {
    // Quilt reads quilt.mod.json before fabric.mod.json
    let Some(descriptor) = relevant.iter().find(|descriptor| descriptor.loader == context.loader).or(relevant.first()) else {
        return Vec::new();
    };
    let mut missing = Vec::new();
    for requirement in &descriptor.requires {
        let id = requirement.id.to_ascii_lowercase();
        if PLATFORM_IDS.contains(&id.as_str()) {
            continue;
        }
        let wanted = (!requirement.versions.is_empty()).then(|| requirement.versions.join(" | "));
        match context.providers.get(&id) {
            None => missing.push(MissingDependency { id: requirement.id.clone(), wanted, installed: None }),
            Some(versions) if !requirement.versions.is_empty() => {
                // unknown versions count as fitting; only a known mismatch of every provider is reported
                let wrong = |version: &Option<String>| version.as_deref().and_then(|text| satisfies(descriptor.loader, &requirement.versions, text)) == Some(false);
                if versions.iter().all(wrong) {
                    let installed = versions.iter().flatten().cloned().collect::<Vec<_>>().join(", ");
                    missing.push(MissingDependency { id: requirement.id.clone(), wanted, installed: Some(installed) });
                }
            }
            Some(_) => {}
        }
    }
    missing
}

fn satisfies(loader: Loader, alternatives: &[String], installed: &str) -> Option<bool> {
    let version = Version::parse(installed)?;
    any_of(alternatives.iter().map(|alternative| match loader {
        Loader::Fabric | Loader::Quilt => matches_semver(alternative, &version),
        Loader::Forge | Loader::NeoForge => matches_maven_range(alternative, &version),
    }))
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
    descriptors_of_archive(&mut zip::ZipArchive::new(reader)?, 0)
}

fn descriptors_of_archive<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, depth: u8) -> Result<Vec<Descriptor>> {
    let mut found = Vec::new();
    if let Some(bytes) = entry(archive, "fabric.mod.json")? {
        let json: Value = serde_json::from_slice(&bytes)?;
        let mut provides = own_ids(&json["id"], &json["version"], &json["provides"]);
        let nested = json["jars"].as_array().into_iter().flatten().filter_map(|jar| jar["file"].as_str().map(str::to_owned)).collect();
        provides.extend(nested_provides(archive, nested, depth));
        let requires = json["depends"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, versions)| Requirement { id: id.clone(), versions: json_alternatives(versions) })
            .collect();
        found.push(Descriptor { loader: Loader::Fabric, minecraft: json_alternatives(&json["depends"]["minecraft"]), provides, requires });
    }
    if let Some(bytes) = entry(archive, "quilt.mod.json")? {
        let json: Value = serde_json::from_slice(&bytes)?;
        let quilt = &json["quilt_loader"];
        let mut provides = own_ids(&quilt["id"], &quilt["version"], &quilt["provides"]);
        let nested = quilt["jars"].as_array().into_iter().flatten().filter_map(|jar| jar.as_str().map(str::to_owned)).collect();
        provides.extend(nested_provides(archive, nested, depth));
        // an object is one dependency; an array of alternatives and an `unless` condition cannot be judged
        let required = quilt["depends"].as_array().into_iter().flatten().filter(|dependency| dependency["optional"] != true && dependency.get("unless").is_none());
        let mut minecraft = Vec::new();
        let mut requires = Vec::new();
        for dependency in required {
            let (id, versions) = match dependency {
                Value::String(id) => (id.as_str(), Vec::new()),
                Value::Object(_) => (dependency["id"].as_str().unwrap_or_default(), json_alternatives(&dependency["versions"])),
                _ => continue,
            };
            if id == "minecraft" {
                minecraft.extend(versions.iter().cloned());
            }
            requires.push(Requirement { id: plain_id(id).to_owned(), versions });
        }
        found.push(Descriptor { loader: Loader::Quilt, minecraft, provides, requires });
    }
    let jarjar: Vec<String> = archive.file_names().filter_map(|name| name.ok()).filter(|name| name.starts_with("META-INF/jarjar/") && name.ends_with(".jar")).map(|name| name.into_owned()).collect();
    for (path, loader) in [("META-INF/neoforge.mods.toml", Loader::NeoForge), ("META-INF/mods.toml", Loader::Forge)] {
        if let Some(bytes) = entry(archive, path)? {
            let mut descriptor = mods_toml_descriptor(&bytes, loader)?;
            descriptor.provides.extend(nested_provides(archive, jarjar.clone(), depth));
            found.push(descriptor);
        }
    }
    if !found.iter().any(|descriptor| matches!(descriptor.loader, Loader::Forge | Loader::NeoForge))
        && let Some(bytes) = entry(archive, "mcmod.info")?
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
        found.push(Descriptor { loader: Loader::Forge, minecraft, provides: Vec::new(), requires: Vec::new() });
    }
    Ok(found)
}

/// Quilt ids may carry a maven group (`org.quiltmc:qsl`); only the id itself is compared.
fn plain_id(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// The mod's own id and its `provides` (strings, or Quilt's `{id, version}`), which share its version unless they state one.
fn own_ids(id: &Value, version: &Value, provides: &Value) -> Vec<Provided> {
    let version = version.as_str().map(str::to_owned);
    let mut ids: Vec<Provided> = id.as_str().into_iter().map(|id| Provided { id: plain_id(id).to_owned(), version: version.clone() }).collect();
    for provided in provides.as_array().into_iter().flatten() {
        match provided {
            Value::String(id) => ids.push(Provided { id: plain_id(id).to_owned(), version: version.clone() }),
            Value::Object(_) => {
                if let Some(id) = provided["id"].as_str() {
                    ids.push(Provided { id: plain_id(id).to_owned(), version: provided["version"].as_str().map(str::to_owned).or_else(|| version.clone()) });
                }
            }
            _ => {}
        }
    }
    ids
}

/// What the jars inside this one provide (Fabric API ships its modules this way). A nested jar that
/// cannot be read provides nothing: the outer jar still works.
fn nested_provides<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, files: Vec<String>, depth: u8) -> Vec<Provided> {
    if depth >= MAX_NESTED_DEPTH {
        return Vec::new();
    }
    let mut provided = Vec::new();
    for file in files {
        let nested = entry(archive, &file).and_then(|bytes| match bytes {
            Some(bytes) => descriptors_of_archive(&mut zip::ZipArchive::new(Cursor::new(bytes))?, depth + 1),
            None => Ok(Vec::new()),
        });
        match nested {
            Ok(descriptors) => provided.extend(descriptors.into_iter().flat_map(|descriptor| descriptor.provides)),
            Err(error) => tracing::warn!("cannot read the nested jar {file}: {error}"),
        }
    }
    provided
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

/// Mod ids, required dependencies, and the `versionRange` of every required dependency on `minecraft`.
fn mods_toml_descriptor(bytes: &[u8], loader: Loader) -> Result<Descriptor> {
    let text = std::str::from_utf8(bytes).map_err(|error| Error::Other(format!("mods.toml is not UTF-8: {error}")))?;
    let root: toml::Table = toml::from_str(text).map_err(|error| Error::Other(format!("invalid mods.toml: {error}")))?;
    let provides = root
        .get("mods")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry.get("modId")?.as_str()?;
            Some(Provided { id: id.to_owned(), version: entry.get("version").and_then(|value| value.as_str()).map(str::to_owned) })
        })
        .collect();
    let mut minecraft = Vec::new();
    let mut requires = Vec::new();
    let tables = root.get("dependencies").and_then(|value| value.as_table()).into_iter().flat_map(|table| table.values());
    for dependency in tables.filter_map(|value| value.as_array()).flatten() {
        let text = |key: &str| dependency.get(key).and_then(|value| value.as_str());
        let optional = dependency.get("mandatory").and_then(|value| value.as_bool()) == Some(false)
            || matches!(text("type"), Some("optional" | "discouraged" | "incompatible"));
        let Some(id) = text("modId") else {
            continue;
        };
        if optional || text("side") == Some("SERVER") {
            continue;
        }
        let versions: Vec<String> = text("versionRange").filter(|range| !range.is_empty()).map(str::to_owned).into_iter().collect();
        if id == "minecraft" {
            minecraft.extend(versions.iter().cloned());
        }
        requires.push(Requirement { id: id.to_owned(), versions });
    }
    Ok(Descriptor { loader, minecraft, provides, requires })
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

    use sha1::Digest;

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

    fn context(minecraft: &str, loader: Loader, has_connector: bool) -> Context {
        Context { minecraft: minecraft.into(), loader, has_connector, providers: HashMap::new() }
    }

    #[test]
    fn fabric_api_for_1_21_11_does_not_fit_1_20_1() {
        for predicate in ["~1.21.11-", ">=1.21.11- <1.21.12-"] {
            let text = format!(r#"{{"id":"fabric-api","version":"0.141.4+1.21.11","depends":{{"fabricloader":">=0.18.1","minecraft":"{predicate}"}}}}"#);
            let descriptors = descriptors_of_jar(jar(&[("fabric.mod.json", &text)])).unwrap();
            let found = judge("fabric-api-0.141.4+1.21.11.jar".into(), &descriptors, &context("1.20.1", Loader::Fabric, false), None).unwrap();
            assert!(found.minecraft_mismatch && found.needs.as_deref() == Some(predicate) && found.source == Some(MismatchSource::Descriptor));
            assert!(judge("fabric-api.jar".into(), &descriptors, &context("1.21.11", Loader::Fabric, false), None).is_none());
        }
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

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn jar(entries: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let entries: Vec<(&str, &[u8])> = entries.iter().map(|(name, text)| (*name, text.as_bytes())).collect();
        Cursor::new(zip_bytes(&entries))
    }

    #[test]
    fn descriptors_of_each_loader() {
        let minecraft = |descriptors: Vec<Descriptor>| descriptors.into_iter().map(|descriptor| (descriptor.loader, descriptor.minecraft)).collect::<Vec<_>>();
        let fabric = descriptors_of_jar(jar(&[("fabric.mod.json", r#"{"id":"a","depends":{"minecraft":["1.20.1","1.20.4"]}}"#)])).unwrap();
        assert_eq!(minecraft(fabric), [(Loader::Fabric, vec!["1.20.1".to_owned(), "1.20.4".to_owned()])]);
        let quilt = descriptors_of_jar(jar(&[("quilt.mod.json", r#"{"quilt_loader":{"depends":[{"id":"minecraft","versions":">=1.20"},"other"]}}"#)])).unwrap();
        assert_eq!(quilt[0].minecraft, [">=1.20"]);
        let quilt_any = descriptors_of_jar(jar(&[("quilt.mod.json", r#"{"quilt_loader":{"depends":[{"id":"minecraft","versions":{"any":["1.20.1","~1.21.11-"]}}]}}"#)])).unwrap();
        assert_eq!(quilt_any[0].minecraft, ["1.20.1", "~1.21.11-"]);
        let toml = "[[mods]]\nmodId=\"a\"\n[[dependencies.a]]\nmodId=\"forge\"\nversionRange=\"[47,)\"\n[[dependencies.a]]\nmodId=\"minecraft\"\nmandatory=true\nversionRange=\"[1.20.1,1.21)\"\n";
        let forge = descriptors_of_jar(jar(&[("META-INF/mods.toml", toml)])).unwrap();
        assert_eq!(minecraft(forge), [(Loader::Forge, vec!["[1.20.1,1.21)".to_owned()])]);
        let optional = toml.replace("mandatory=true", "mandatory=false");
        assert!(descriptors_of_jar(jar(&[("META-INF/mods.toml", &optional)])).unwrap()[0].minecraft.is_empty());
        let legacy = descriptors_of_jar(jar(&[("mcmod.info", r#"[{"modid":"a","name":"A","mcversion":"1.12.2"}]"#)])).unwrap();
        assert_eq!(minecraft(legacy), [(Loader::Forge, vec!["1.12.x".to_owned()])]);
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

    fn descriptor(loader: Loader, spec: &[&str]) -> Descriptor {
        Descriptor { loader, minecraft: spec.iter().map(|s| (*s).to_owned()).collect(), provides: Vec::new(), requires: Vec::new() }
    }

    fn fabric(spec: &[&str]) -> Descriptor {
        descriptor(Loader::Fabric, spec)
    }

    #[test]
    fn judging_version_and_loader() {
        let verdict = |descriptors: &[Descriptor], loader| judge("m.jar".into(), descriptors, &context("1.21.4", loader, false), None);
        let wrong = verdict(&[fabric(&["1.20.1"])], Loader::Fabric).unwrap();
        assert!(wrong.minecraft_mismatch && wrong.needs.as_deref() == Some("1.20.1") && wrong.enabled);
        assert!(verdict(&[fabric(&["1.21.x"])], Loader::Fabric).is_none());
        assert!(verdict(&[fabric(&[])], Loader::Fabric).is_none());
        assert!(verdict(&[fabric(&["1.20.1", "garbage"])], Loader::Fabric).is_none());
        // Quilt runs Fabric mods
        assert!(verdict(&[fabric(&["1.21.4"])], Loader::Quilt).is_none());
        let forge_only = [descriptor(Loader::Forge, &["[1.20.1]"])];
        let loader = verdict(&forge_only, Loader::Fabric).unwrap();
        assert!(loader.loader_mismatch && !loader.minecraft_mismatch && loader.made_for_loaders == [Loader::Forge]);
        // a jar for several loaders is fine when one fits, judged by that one
        let both = [forge_only[0].clone(), fabric(&["1.21.4"])];
        assert!(verdict(&both, Loader::Fabric).is_none());
        assert!(judge("m.jar".into(), &[fabric(&[])], &context("1.21.4", Loader::Forge, true), None).is_none());
        assert!(judge("m.jar".into(), &[fabric(&[])], &context("1.21.4", Loader::Forge, false), None).unwrap().loader_mismatch);
        assert!(!judge("m.jar.disabled".into(), &[fabric(&["1.20.1"])], &context("1.21.4", Loader::Fabric, false), None).unwrap().enabled);
        assert!(verdict(&[], Loader::Fabric).is_none());
    }

    #[test]
    fn modrinth_data_judges_jars_with_a_silent_descriptor() {
        let info = |versions: &[&str]| ModrinthInfo { game_versions: versions.iter().map(|v| (*v).to_owned()).collect() };
        let verdict = |versions: &[&str], minecraft: &str, descriptors: &[Descriptor]| {
            judge("iris-fabric-1.10.7+mc1.21.11.jar".into(), descriptors, &context(minecraft, Loader::Fabric, false), Some(&info(versions)))
        };
        let wrong = verdict(&["1.21.11"], "1.20.1", &[fabric(&[])]).unwrap();
        assert!(wrong.minecraft_mismatch && wrong.needs.as_deref() == Some("1.21.11") && wrong.source == Some(MismatchSource::Modrinth));
        assert!(verdict(&["1.20.1", "1.20.4"], "1.20.1", &[fabric(&[])]).is_none());
        // "1.21" and "1.21.0" are the same release
        assert!(verdict(&["1.21"], "1.21.0", &[fabric(&[])]).is_none());
        assert!(verdict(&[], "1.20.1", &[fabric(&[])]).is_none());
        let many = verdict(&["1.21", "1.21.1", "1.21.2", "1.21.3", "1.21.4"], "1.20.1", &[fabric(&[])]).unwrap();
        assert_eq!(many.needs.as_deref(), Some("1.21, 1.21.1, 1.21.2, ..."));
        // a descriptor that names Minecraft is the stronger word, Modrinth is not consulted
        assert!(verdict(&["1.21.11"], "1.20.1", &[fabric(&["~1.20.1"])]).is_none());
        // a file that Modrinth knows is not judged by its name
        assert!(verdict(&["1.20.1"], "1.20.1", &[fabric(&[])]).is_none());
    }

    #[test]
    fn file_names_are_a_weak_hint() {
        let name = |file: &str, minecraft: &str| filename_minecraft(file, minecraft);
        assert_eq!(name("iris-fabric-1.10.7+mc1.21.11.jar", "1.20.1").as_deref(), Some("1.21.11"));
        assert_eq!(name("sodium-fabric-0.5.3+mc1.20.1.jar", "1.20.1"), None);
        assert_eq!(name("fabric-api-0.92.12+1.20.1.jar", "1.21.4").as_deref(), Some("1.20.1"));
        assert_eq!(name("mod-mc1.20.4-1.0.jar.disabled", "1.21").as_deref(), Some("1.20.4"));
        // the same release line is not a clear difference
        assert_eq!(name("iris-1.10.7+mc1.21.11.jar", "1.21.4"), None);
        // a bare version is the mod's own
        assert_eq!(name("citresewn-1.20.1.jar", "1.21.4"), None);
        assert_eq!(name("mod-1.21.11.jar", "1.20.1"), None);
        assert_eq!(name("mod-1.0+1.2.jar", "1.20.1"), None);
        // one token that matches is enough
        assert_eq!(name("mod-mc1.20.1-mc1.21.jar", "1.21.2"), None);
        // unknown instance versions are never judged
        assert_eq!(name("mod+mc1.21.11.jar", "24w14a"), None);
        assert_eq!(name("mod+mc1.21.11.jar", "1.21-pre1"), None);
        assert_eq!(name("mod+mc26.1.jar", "1.21.4").as_deref(), Some("26.1"));
    }

    struct Env {
        _dir: tempfile::TempDir,
        game: PathBuf,
        instance: PathBuf,
        incoming: PathBuf,
    }

    impl Env {
        fn new() -> Env {
            let dir = tempfile::tempdir().unwrap();
            let (game, instance, incoming) = (dir.path().join("game"), dir.path().join("inst"), dir.path().join("incoming"));
            for folder in [game.join("mods"), instance.clone(), incoming.clone()] {
                std::fs::create_dir_all(folder).unwrap();
            }
            Env { _dir: dir, game, instance, incoming }
        }

        fn install(&self, name: &str, bytes: &[u8]) -> String {
            std::fs::write(self.game.join("mods").join(name), bytes).unwrap();
            hex::encode(sha1::Sha1::digest(bytes))
        }

        fn fabric(&self, name: &str, json: &str) -> String {
            self.install(name, &zip_bytes(&[("fabric.mod.json", json.as_bytes())]))
        }

        fn add(&self, name: &str, json: &str) -> PathBuf {
            let path = self.incoming.join(name);
            std::fs::write(&path, zip_bytes(&[("fabric.mod.json", json.as_bytes())])).unwrap();
            path
        }

        /// `known`: what the mocked Modrinth answers for a SHA-1; `None`: the request fails. Returns the verdicts
        /// and the hashes asked about, per request.
        async fn run(&self, minecraft: &str, incoming: Option<Vec<PathBuf>>, known: Option<&[(&str, &[&str])]>) -> (Vec<ModCompat>, Vec<Vec<String>>) {
            let requests = std::cell::RefCell::new(Vec::new());
            let found = check(
                async |hashes: Vec<String>| {
                    requests.borrow_mut().push(hashes.clone());
                    let known = known.ok_or_else(|| Error::Other("offline".into()))?;
                    Ok(known
                        .iter()
                        .filter(|(hash, _)| hashes.iter().any(|asked| asked == hash))
                        .map(|(hash, versions)| ((*hash).to_owned(), ModrinthInfo { game_versions: versions.iter().map(|v| (*v).to_owned()).collect() }))
                        .collect())
                },
                &self.game,
                &self.instance,
                minecraft,
                Some(Loader::Fabric),
                incoming,
            )
            .await
            .unwrap();
            (found, requests.into_inner())
        }

        fn cache(&self) -> Cache {
            serde_json::from_slice(&std::fs::read(self.instance.join(CACHE_FILE)).unwrap()).unwrap()
        }
    }

    #[tokio::test]
    async fn unreadable_jars_are_not_cached() {
        let env = Env::new();
        env.install("half.jar", b"not a zip");
        env.fabric("good.jar", r#"{"id":"g"}"#);
        env.run("1.21.4", None, Some(&[])).await;
        let cache = env.cache();
        assert_eq!(cache.descriptors.len(), 1);
        assert!(cache.descriptors.keys().all(|key| key.starts_with("good.jar|")));
    }

    #[tokio::test]
    async fn check_mods_reads_caches_and_reports() {
        let env = Env::new();
        env.fabric("old.jar", r#"{"id":"o","depends":{"minecraft":"~1.20.1"}}"#);
        env.fabric("ok.jar", r#"{"id":"k","depends":{"minecraft":">=1.21"}}"#);
        env.install("lib.jar", b"not a zip");
        std::fs::create_dir(env.game.join("mods").join("folder.jar")).unwrap();
        let (found, requests) = env.run("1.21.4", None, Some(&[])).await;
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].file_name.as_str(), found[0].needs.as_deref()), ("old.jar", Some("~1.20.1")));
        // both descriptors name Minecraft, so Modrinth is not asked
        assert!(requests.is_empty());
        assert!(env.instance.join(CACHE_FILE).exists());
        // the second run is answered from the cache
        assert_eq!(env.run("1.21.4", None, Some(&[])).await.0, found);
        let none = check(async |_| Ok(HashMap::new()), &env.game, &env.instance, "1.21.4", None, None).await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn iris_for_1_21_11_is_caught_in_1_20_1() {
        let env = Env::new();
        let iris = r#"{"id":"iris","version":"1.10.7+mc1.21.11","depends":{"fabricloader":">=0.12.3","sodium":["0.8.x"]}}"#;
        let iris_hash = env.fabric("iris-fabric-1.10.7+mc1.21.11.jar", iris);
        env.fabric("sodium-0.5.3.jar", r#"{"id":"sodium","version":"0.5.3+mc1.20.1","depends":{"minecraft":"~1.20.1"}}"#);
        env.fabric("citresewn.jar", r#"{"id":"citresewn","version":"1.2.2","depends":{"fabricloader":">=0.14"}}"#);
        let (found, requests) = env.run("1.20.1", None, Some(&[(&iris_hash, &["1.21.11"])])).await;
        // one request for the jars with a silent descriptor
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].len(), 2);
        let names: Vec<_> = found.iter().map(|compat| compat.file_name.as_str()).collect();
        assert_eq!(names, ["iris-fabric-1.10.7+mc1.21.11.jar"]);
        let iris = &found[0];
        assert_eq!((iris.source, iris.needs.as_deref()), (Some(MismatchSource::Modrinth), Some("1.21.11")));
        assert_eq!(iris.dependencies, [MissingDependency { id: "sodium".into(), wanted: Some("0.8.x".into()), installed: Some("0.5.3+mc1.20.1".into()) }]);

        // answers, including "not found", are cached and not asked again
        let (again, requests) = env.run("1.20.1", None, Some(&[])).await;
        assert!(requests.is_empty());
        assert_eq!(again, found);
        let cache = env.cache();
        assert_eq!(cache.modrinth.len(), 2);
        assert_eq!(cache.modrinth.values().flatten().count(), 1);
    }

    #[tokio::test]
    async fn a_failed_lookup_gives_no_verdict_and_is_not_cached() {
        let env = Env::new();
        env.fabric("iris-fabric-1.10.7+mc1.21.11.jar", r#"{"id":"iris","depends":{"fabricloader":">=0.12.3"}}"#);
        let (found, requests) = env.run("1.20.1", None, None).await;
        assert_eq!(requests.len(), 1);
        // only the file name is left to go by
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, Some(MismatchSource::Filename));
        assert!(env.cache().modrinth.is_empty());
        let (_, requests) = env.run("1.20.1", None, None).await;
        assert_eq!(requests.len(), 1);
        // once Modrinth answers that it does not know the file, that is remembered as well
        let (found, _) = env.run("1.20.1", None, Some(&[])).await;
        assert_eq!(found[0].source, Some(MismatchSource::Filename));
        assert_eq!(env.cache().modrinth.values().collect::<Vec<_>>(), [&None]);
        assert!(env.run("1.20.1", None, None).await.1.is_empty());
    }

    #[tokio::test]
    async fn dependencies_come_from_ids_provides_and_nested_jars() {
        let env = Env::new();
        let nested = zip_bytes(&[("fabric.mod.json", br#"{"id":"fabric-resource-loader-v0","version":"1.0.0"}"#)]);
        let api = zip_bytes(&[
            ("fabric.mod.json", br#"{"id":"fabric-api","version":"0.92.12+1.20.1","provides":["fabric"],"jars":[{"file":"META-INF/jars/loader.jar"}]}"#),
            ("META-INF/jars/loader.jar", &nested),
        ]);
        env.install("fabric-api.jar", &api);
        let needy = r#"{"id":"needy","depends":{"fabricloader":">=0.14","minecraft":"1.20.1","java":">=17","fabric":"*","fabric-resource-loader-v0":">=0.9","fabric-api":">=0.90"}}"#;
        env.fabric("needy.jar", needy);
        assert!(env.run("1.20.1", None, Some(&[])).await.0.is_empty());

        // without Fabric API all three ids are missing; the platform ids never are
        std::fs::rename(env.game.join("mods/fabric-api.jar"), env.game.join("mods/fabric-api.jar.disabled")).unwrap();
        let found = env.run("1.20.1", None, Some(&[])).await.0;
        let missing: Vec<_> = found[0].dependencies.iter().map(|dependency| dependency.id.as_str()).collect();
        assert_eq!(missing, ["fabric", "fabric-api", "fabric-resource-loader-v0"]);
        // the disabled jar itself is not judged for its dependencies
        assert!(found.len() == 1 && found[0].file_name == "needy.jar");

        // a version predicate on a nested module
        std::fs::rename(env.game.join("mods/fabric-api.jar.disabled"), env.game.join("mods/fabric-api.jar")).unwrap();
        env.fabric("needy.jar", &needy.replace("\">=0.9\"", "\">=2.0\""));
        let found = env.run("1.20.1", None, Some(&[])).await.0;
        assert_eq!(found[0].dependencies, [MissingDependency { id: "fabric-resource-loader-v0".into(), wanted: Some(">=2.0".into()), installed: Some("1.0.0".into()) }]);
    }

    #[tokio::test]
    async fn a_dependency_of_unknown_version_is_not_reported() {
        let env = Env::new();
        env.fabric("lib.jar", r#"{"id":"lib","version":"weird-version"}"#);
        env.fabric("user.jar", r#"{"id":"user","depends":{"lib":">=5.0","optional-looking":"*"},"recommends":{"x":"*"},"suggests":{"y":"*"},"breaks":{"z":"*"}}"#);
        let found = env.run("1.20.1", None, Some(&[])).await.0;
        let missing: Vec<_> = found[0].dependencies.iter().map(|dependency| dependency.id.as_str()).collect();
        assert_eq!(missing, ["optional-looking"]);
    }

    #[test]
    fn quilt_and_forge_dependencies() {
        let quilt = descriptors_of_jar(jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"q","version":"1","provides":["alias",{"id":"other","version":"2"}],"depends":["org.quiltmc:qsl",{"id":"qfapi","versions":">=1"},{"id":"soft","optional":true},{"id":"x","unless":"y"},{"id":"minecraft","versions":">=1.20"}]}}"#,
        )]))
        .unwrap();
        let ids: Vec<_> = quilt[0].requires.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["qsl", "qfapi", "minecraft"]);
        let provided: Vec<_> = quilt[0].provides.iter().map(|p| (p.id.as_str(), p.version.as_deref())).collect();
        assert_eq!(provided, [("q", Some("1")), ("alias", Some("1")), ("other", Some("2"))]);

        let toml = r#"
[[mods]]
modId="a"
version="1.0.0"
[[mods]]
modId="b"
[[dependencies.a]]
modId="minecraft"
mandatory=true
versionRange="[1.20.1]"
[[dependencies.a]]
modId="lib"
mandatory=true
versionRange="[2,3)"
[[dependencies.a]]
modId="soft"
mandatory=false
[[dependencies.a]]
modId="neo"
type="required"
[[dependencies.a]]
modId="serveronly"
type="required"
side="SERVER"
[[dependencies.a]]
modId="nope"
type="incompatible"
"#;
        let forge = descriptors_of_jar(jar(&[("META-INF/mods.toml", toml), ("META-INF/jarjar/metadata.json", "{}")])).unwrap();
        let ids: Vec<_> = forge[0].requires.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["minecraft", "lib", "neo"]);
        assert_eq!(forge[0].provides.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);

        // judged: lib 1.5 does not satisfy [2,3), neo is absent, platform ids are skipped
        let mut context = context("1.20.1", Loader::Forge, false);
        context.providers.insert("lib".into(), vec![Some("1.5".into())]);
        let missing = missing_dependencies(&[&forge[0]], &context);
        assert_eq!(missing.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["lib", "neo"]);
        assert_eq!(missing[0].installed.as_deref(), Some("1.5"));
        context.providers.insert("lib".into(), vec![Some("2.1".into())]);
        assert_eq!(missing_dependencies(&[&forge[0]], &context).len(), 1);
    }

    #[tokio::test]
    async fn new_jars_are_judged_with_the_installed_ones_and_each_other() {
        let env = Env::new();
        env.fabric("base.jar", r#"{"id":"base","version":"1.0.0"}"#);
        let old = env.add("old.jar", r#"{"id":"o","depends":{"minecraft":"1.20.1"}}"#);
        let ok = env.add("ok.jar", r#"{"id":"k","depends":{"minecraft":"1.21.x","base":">=1","extra":"*"}}"#);
        let extra = env.add("extra.jar", r#"{"id":"extra","depends":{"minecraft":"1.21.x","missing":"*"}}"#);
        let broken = env.incoming.join("broken.jar");
        std::fs::write(&broken, b"not a zip").unwrap();
        let pack = env.incoming.join("pack.zip");
        std::fs::write(&pack, b"").unwrap();
        let (found, requests) = env.run("1.21.4", Some(vec![old, ok, extra, broken, pack]), Some(&[])).await;
        assert!(requests.is_empty());
        let verdicts: Vec<_> = found.iter().map(|compat| (compat.file_name.as_str(), compat.needs.as_deref(), compat.dependencies.len())).collect();
        // ok.jar finds `base` installed and `extra` among the incoming files; extra.jar lacks `missing`
        assert_eq!(verdicts, [("extra.jar", None, 1), ("old.jar", Some("1.20.1"), 0)]);
        // the game folder may not have a mods folder yet
        std::fs::remove_dir_all(env.game.join("mods")).unwrap();
        let found = env.run("1.21.4", Some(vec![env.incoming.join("old.jar")]), Some(&[])).await.0;
        assert_eq!(found.len(), 1);
    }

    /// Tauri commands must be `Send`; a generic `AsyncFnOnce` could silently break that.
    #[test]
    fn the_public_checks_are_send() {
        fn assert_send<T: Send>(_: T) {}
        let client = modrinth::Client::new(reqwest::Client::new());
        assert_send(check_mods(&client, Path::new("."), Path::new("."), "1.20.1", None));
        assert_send(check_new_jars(&client, Path::new("."), Path::new("."), Vec::new(), "1.20.1", None));
    }

    #[tokio::test]
    async fn loaders_are_required_for_any_verdict() {
        let env = Env::new();
        env.fabric("a.jar", r#"{"id":"a","depends":{"missing":"*"}}"#);
        assert!(check(async |_| Ok(HashMap::new()), &env.game, &env.instance, "1.21.4", None, None).await.unwrap().is_empty());
    }
}
