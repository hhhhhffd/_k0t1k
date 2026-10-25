//! Wasm plugins (RatScan, packconv): signed releases of their own GitHub repositories, installed under
//! `<data>/plugins/<id>/<version>/`. The launcher also ships a baseline of each plugin; running them
//! is the UI's job (a dedicated Web Worker), this module only finds, verifies, installs and reads files.
//!
//! A release has the assets `plugin.json`, `plugin.json.sig` and every file listed in the manifest.
//! The signature is the base64 of a raw Ed25519 signature over [`manifest_message`] (a fixed prefix and
//! the exact bytes of `plugin.json`). The manifest carries the SHA-256 of every file, so one signature
//! covers the whole release. The plugin key is not the launcher self-update key. Any failure leaves the
//! active version as it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;
use crate::update;

/// The export contract (`default` init, `scan`, `convert`, ...) this launcher can call.
pub const API: u64 = 1;
/// Raw 32-byte Ed25519 public key, base64. The private half is kept off every repository.
const PUBLIC_KEY: &str = "b37Axim0D7VgDNWVIgvlnpdn2nVOMSLM/jIFbRFc/4k=";
const MESSAGE_PREFIX: &[u8] = b"_k0t1k-plugin\n";
const MANIFEST_ASSET: &str = "plugin.json";
const SIGNATURE_ASSET: &str = "plugin.json.sig";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;
/// A wasm plugin is a few MB; a file far beyond that is not ours.
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 8;
/// How often the startup check may ask GitHub.
pub const CHECK_INTERVAL: Duration = Duration::hours(24);

/// A plugin the launcher knows. Releases are read only from this repository.
#[derive(Debug, Clone, Copy)]
pub struct KnownPlugin {
    pub id: &'static str,
    /// `owner/name`.
    pub repo: &'static str,
}

impl KnownPlugin {
    fn asset_prefix(&self) -> String {
        format!("https://github.com/{}/releases/download/", self.repo)
    }
}

/// `plugin.json`, in releases, installed folders and the bundled baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub api: u64,
    pub min_launcher: String,
    /// File name to lowercase hex SHA-256.
    pub files: BTreeMap<String, String>,
}

/// The bytes the plugin key signs.
pub fn manifest_message(manifest: &[u8]) -> Vec<u8> {
    [MESSAGE_PREFIX, manifest].concat()
}

fn reject(what: impl std::fmt::Display) -> Error {
    Error::Other(format!("plugin rejected: {what}"))
}

/// Plain file names only: nothing in a manifest may reach outside the plugin's folder.
fn valid_file_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Checks everything that does not need the files: signature, id, api, minimum launcher, file list shape.
fn verify_manifest_with(public_key: &str, manifest: &[u8], signature_b64: &str, plugin: &KnownPlugin, launcher: &str) -> Result<Manifest> {
    update::verify_with(public_key, &manifest_message(manifest), signature_b64)?;
    let parsed: Manifest = serde_json::from_slice(manifest).map_err(|error| reject(format!("bad plugin.json: {error}")))?;
    if parsed.id != plugin.id {
        return Err(reject(format!("it is {:?}, expected {:?}", parsed.id, plugin.id)));
    }
    if parsed.api != API {
        return Err(reject(format!("it needs plugin api {}, this launcher has {API}", parsed.api)));
    }
    let wanted = update::parse_version(&parsed.min_launcher).ok_or_else(|| reject(format!("bad minLauncher {:?}", parsed.min_launcher)))?;
    let running = update::parse_version(launcher).ok_or_else(|| reject(format!("bad launcher version {launcher:?}")))?;
    if wanted > running {
        return Err(reject(format!("it needs launcher {}, this is {launcher}", parsed.min_launcher)));
    }
    if update::parse_version(&parsed.version).is_none() {
        return Err(reject(format!("bad version {:?}", parsed.version)));
    }
    if parsed.files.is_empty() || parsed.files.len() > MAX_FILES {
        return Err(reject("unexpected number of files"));
    }
    for (name, hash) in &parsed.files {
        let hex = hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid_file_name(name) || !hex {
            return Err(reject(format!("bad file entry {name:?}")));
        }
    }
    Ok(parsed)
}

/// A downloaded release is acceptable when its manifest is signed and valid, it is the tagged version
/// and it is newer than `current`.
fn verify_release_with(
    public_key: &str,
    manifest: &[u8],
    signature_b64: &str,
    plugin: &KnownPlugin,
    launcher: &str,
    tag: &str,
    current: &str,
) -> Result<Manifest> {
    let parsed = verify_manifest_with(public_key, manifest, signature_b64, plugin, launcher)?;
    let tagged = update::parse_version(tag).map(update::normalized);
    if tagged.as_deref() != Some(parsed.version.as_str()) {
        return Err(reject(format!("tag {tag:?} does not match version {}", parsed.version)));
    }
    if !update::is_newer(&parsed.version, current)? {
        return Err(reject(format!("{} is not newer than {current}", parsed.version)));
    }
    Ok(parsed)
}

/// Where `name` of the release `tag` is downloaded from: always under the plugin's own release downloads
/// (`valid_file_name` has been checked for every manifest entry, so `name` cannot leave that folder).
fn asset_url(plugin: &KnownPlugin, tag: &str, name: &str) -> String {
    format!("{}{tag}/{name}", plugin.asset_prefix())
}

/// Checks `bytes` against the hash the manifest lists for `name`.
pub fn verify_file(manifest: &Manifest, name: &str, bytes: &[u8]) -> Result<()> {
    let expected = manifest.files.get(name).ok_or_else(|| reject(format!("{name} is not in plugin.json")))?;
    let actual = hex::encode(Sha256::digest(bytes));
    if &actual != expected {
        return Err(reject(format!("{name} has sha256 {actual}, plugin.json says {expected}")));
    }
    Ok(())
}

// ---- installed versions ----

fn plugin_dir(paths: &Paths, plugin: &KnownPlugin) -> PathBuf {
    paths.plugins_dir().join(plugin.id)
}

/// An installed version whose signature and file hashes were checked when it was loaded.
#[derive(Debug, Clone)]
pub struct Installed {
    pub manifest: Manifest,
    dir: PathBuf,
}

impl Installed {
    /// Reads a file of the plugin and checks it against the manifest again.
    pub fn read_file(&self, name: &str) -> Result<Vec<u8>> {
        let path = self.dir.join(name);
        let bytes = std::fs::read(&path).at(&path)?;
        verify_file(&self.manifest, name, &bytes)?;
        Ok(bytes)
    }
}

/// Loads `<id>/<version>`: the stored manifest must still be signed and every file must still hash right.
fn load_installed_with(public_key: &str, paths: &Paths, plugin: &KnownPlugin, version: &str, launcher: &str) -> Result<Installed> {
    let version = update::parse_version(version).map(update::normalized).ok_or_else(|| reject(format!("bad version {version:?}")))?;
    let dir = plugin_dir(paths, plugin).join(&version);
    let read = |name: &str| std::fs::read(dir.join(name)).at(dir.join(name));
    let manifest_bytes = read(MANIFEST_ASSET)?;
    let signature = String::from_utf8(read(SIGNATURE_ASSET)?).map_err(|_| reject("the signature is not text"))?;
    let manifest = verify_manifest_with(public_key, &manifest_bytes, &signature, plugin, launcher)?;
    if manifest.version != version {
        return Err(reject(format!("folder {version} holds version {}", manifest.version)));
    }
    let installed = Installed { manifest, dir };
    for name in installed.manifest.files.keys() {
        installed.read_file(name)?;
    }
    Ok(installed)
}

fn active_version(paths: &Paths, plugin: &KnownPlugin) -> Result<Option<String>> {
    let pointer = plugin_dir(paths, plugin).join("active");
    match std::fs::read_to_string(&pointer) {
        Ok(text) => Ok(Some(text.trim().to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).at(&pointer),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Bundled,
    Installed,
}

/// Which copy of a plugin runs.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub version: String,
    pub source: Source,
    /// Present when `source` is `Installed`.
    pub installed: Option<Installed>,
    /// Why an installed copy was not used although one exists (it is damaged); the bundled one runs.
    pub problem: Option<String>,
}

/// The installed active version when it is valid and not older than the bundled baseline, else the baseline.
pub fn resolve(paths: &Paths, plugin: &KnownPlugin, launcher: &str, bundled_version: &str) -> Resolved {
    let bundled = |problem| Resolved { version: bundled_version.to_owned(), source: Source::Bundled, installed: None, problem };
    let loaded = active_version(paths, plugin)
        .and_then(|version| version.map(|version| load_installed_with(PUBLIC_KEY, paths, plugin, &version, launcher)).transpose());
    match loaded {
        Ok(Some(installed)) if update::is_newer(&installed.manifest.version, bundled_version).unwrap_or(false) || installed.manifest.version == bundled_version => {
            Resolved { version: installed.manifest.version.clone(), source: Source::Installed, installed: Some(installed), problem: None }
        }
        // An installed copy older than the baseline shipped with a newer launcher is simply superseded.
        Ok(_) => bundled(None),
        Err(error) => bundled(Some(error.to_string())),
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).at(path)?;
    file.write_all(bytes).at(path)?;
    file.sync_all().at(path)
}

/// Writes a verified release into `<id>/<version>`, switches `active` to it and deletes older versions
/// except the one that was active before.
fn install(paths: &Paths, plugin: &KnownPlugin, manifest: &Manifest, manifest_bytes: &[u8], signature: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
    let root = plugin_dir(paths, plugin);
    let staging = root.join(format!(".staging-{}", manifest.version));
    let target = root.join(&manifest.version);
    let previous = active_version(paths, plugin).unwrap_or(None);
    remove_dir_if_exists(&staging)?;
    std::fs::create_dir_all(&staging).at(&staging)?;
    let written = (|| {
        for (name, bytes) in files {
            write_synced(&staging.join(name), bytes)?;
        }
        write_synced(&staging.join(MANIFEST_ASSET), manifest_bytes)?;
        write_synced(&staging.join(SIGNATURE_ASSET), signature.as_bytes())?;
        remove_dir_if_exists(&target)?;
        std::fs::rename(&staging, &target).at(&target)?;
        let pointer = root.join("active");
        let pending = root.join("active.tmp");
        write_synced(&pending, manifest.version.as_bytes())?;
        std::fs::rename(&pending, &pointer).at(&pointer)
    })();
    if let Err(error) = written {
        if let Err(cleanup) = remove_dir_if_exists(&staging) {
            tracing::warn!("cannot remove {}: {cleanup}", staging.display());
        }
        return Err(error);
    }
    prune(&root, &manifest.version, previous.as_deref());
    Ok(())
}

fn remove_dir_if_exists(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error).at(dir),
        _ => Ok(()),
    }
}

/// Deletes every version folder (and leftover staging folder) except the active and the previous one.
/// Failures are logged: a leftover folder is harmless.
fn prune(root: &Path, active: &str, previous: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let keep = name == active || Some(name.as_str()) == previous || !entry.path().is_dir();
        if !keep && let Err(error) = std::fs::remove_dir_all(entry.path()) {
            tracing::warn!("cannot remove the old plugin folder {}: {error}", entry.path().display());
        }
    }
}

// ---- check state ----

/// What the last check of a plugin found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginRecord {
    pub last_check: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// The last attempt hit a rate limit or a network problem: not a failure of the plugin, retried sooner.
    pub delayed: bool,
}

pub type PluginStates = BTreeMap<String, PluginRecord>;

fn state_file(paths: &Paths) -> PathBuf {
    paths.plugins_dir().join("state.json")
}

/// A missing or unreadable file is an empty state (and logged): it only decides when to check next.
pub fn load_states(paths: &Paths) -> PluginStates {
    let file = state_file(paths);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!("ignoring {}: {error}", file.display());
            PluginStates::new()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PluginStates::new(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", file.display());
            PluginStates::new()
        }
    }
}

pub fn save_states(paths: &Paths, states: &PluginStates) -> Result<()> {
    let dir = paths.plugins_dir();
    std::fs::create_dir_all(&dir).at(&dir)?;
    let (file, pending) = (state_file(paths), dir.join("state.json.tmp"));
    std::fs::write(&pending, serde_json::to_vec_pretty(states)?).at(&pending)?;
    std::fs::rename(&pending, &file).at(&file)
}

/// Whether the startup check may ask GitHub again: once a day, or after an hour when the last attempt was delayed.
pub fn check_due(record: Option<&PluginRecord>, now: DateTime<Utc>) -> bool {
    let interval = if record.is_some_and(|record| record.delayed) { update::RETRY_AFTER_TRANSIENT } else { CHECK_INTERVAL };
    record.and_then(|record| record.last_check).is_none_or(|last| now - last >= interval)
}

// ---- updating ----

/// Looks for a newer release of `plugin` than `current` and installs it. Returns the new version, or
/// `None` when there is nothing newer (including no release at all).
pub async fn update(client: &reqwest::Client, paths: &Paths, plugin: &KnownPlugin, launcher: &str, current: &str) -> Result<Option<String>> {
    let Some(latest) = update::latest_version(plugin.repo).await? else {
        return Ok(None);
    };
    let tag = format!("v{}", update::normalized(latest));
    if !update::is_newer(&tag, current)? {
        return Ok(None);
    }
    let manifest_bytes = update::fetch_capped(client, &asset_url(plugin, &tag, MANIFEST_ASSET), MAX_MANIFEST_BYTES, "plugin.json").await?;
    let signature = update::fetch_capped(client, &asset_url(plugin, &tag, SIGNATURE_ASSET), MAX_SIGNATURE_BYTES, "signature file").await?;
    let signature = String::from_utf8(signature).map_err(|_| reject("the signature is not text"))?;
    let manifest = verify_release_with(PUBLIC_KEY, &manifest_bytes, &signature, plugin, launcher, &tag, current)?;

    let mut files = Vec::new();
    for name in manifest.files.keys() {
        let bytes = update::fetch_capped(client, &asset_url(plugin, &tag, name), MAX_FILE_BYTES, name).await?;
        verify_file(&manifest, name, &bytes)?;
        files.push((name.clone(), bytes));
    }
    let (paths, plugin_copy, installed_manifest) = (paths.clone(), *plugin, manifest.clone());
    tokio::task::spawn_blocking(move || install(&paths, &plugin_copy, &installed_manifest, &manifest_bytes, &signature, &files))
        .await
        .map_err(|error| Error::Other(format!("plugin install task failed: {error}")))??;
    Ok(Some(manifest.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway key made with openssl for these tests only; the real plugin key is not in the repository.
    // `SIG` signs "_k0t1k-plugin\n" + MANIFEST.
    const KEY: &str = "QJzhCxQHU5Itujfc9fVd7JU96j2u3RZ3wq9D8c7LX0E=";
    const SIG: &str = "SmDyayxTd+R4V+bm4hhxUx61vcm7ky/biS3aiU6jl3nXf2zP4YlLj1xdp3TyyPfMujVQJcSLMFi6e3i2dFd5Bw==";
    const MANIFEST: &str = r#"{"id":"ratscan","version":"0.2.0","api":1,"minLauncher":"0.1.0","files":{"ratscan_wasm.js":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","ratscan_wasm_bg.wasm":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}}"#;
    const SIG_API2: &str = "XfejXMz9x68h/eqNOLwr78xbN31TZzRV2S371f935PsIMo4+GD11CLa+IUTIk7c/OEikSbnFkF8S7ZVzimyFCg==";
    const PLUGIN: KnownPlugin = KnownPlugin { id: "ratscan", repo: "hhhhhffd/ratka_checker" };

    fn verify(manifest: &str, sig: &str, launcher: &str, tag: &str, current: &str) -> Result<Manifest> {
        verify_release_with(KEY, manifest.as_bytes(), sig, &PLUGIN, launcher, tag, current)
    }

    #[test]
    fn valid_release_is_accepted() {
        let manifest = verify(MANIFEST, SIG, "0.1.0", "v0.2.0", "0.1.0").unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.files.len(), 2);
        verify_file(&manifest, "ratscan_wasm.js", b"hello").unwrap();
    }

    #[test]
    fn tampered_manifest_wrong_key_and_bad_signature_are_rejected() {
        let tampered = MANIFEST.replace("0.2.0", "9.9.9");
        assert!(verify(&tampered, SIG, "0.1.0", "v9.9.9", "0.1.0").is_err());
        let other_hash = MANIFEST.replace("2cf24dba", "2cf24dbb");
        assert!(verify(&other_hash, SIG, "0.1.0", "v0.2.0", "0.1.0").is_err());
        // The embedded launcher keys do not accept a signature of the throwaway key.
        assert!(verify_release_with(PUBLIC_KEY, MANIFEST.as_bytes(), SIG, &PLUGIN, "0.1.0", "v0.2.0", "0.1.0").is_err());
        assert!(verify(MANIFEST, "", "0.1.0", "v0.2.0", "0.1.0").is_err());
        assert!(verify(MANIFEST, "!!!", "0.1.0", "v0.2.0", "0.1.0").is_err());
    }

    #[test]
    fn id_api_launcher_tag_and_version_are_checked() {
        let other = KnownPlugin { id: "packconv", repo: "hhhhhffd/conventor-mc-rp" };
        assert!(verify_release_with(KEY, MANIFEST.as_bytes(), SIG, &other, "0.1.0", "v0.2.0", "0.1.0").is_err());
        // api and minLauncher changes need a new signature, so check the inner function with the key's own message.
        assert!(verify(MANIFEST, SIG, "0.0.9", "v0.2.0", "0.1.0").is_err(), "minLauncher too high");
        assert!(verify(MANIFEST, SIG, "0.1.0", "v0.3.0", "0.1.0").is_err(), "tag differs from version");
        assert!(verify(MANIFEST, SIG, "0.1.0", "v0.2.0", "0.2.0").is_err(), "not newer");
        assert!(verify(MANIFEST, SIG, "0.1.0", "v0.2.0", "0.3.0").is_err(), "older");
    }

    #[test]
    fn unknown_api_is_rejected() {
        // The same manifest with "api":2, signed by the throwaway key.
        let api2 = MANIFEST.replace("\"api\":1", "\"api\":2");
        assert!(verify(&api2, SIG_API2, "0.1.0", "v0.2.0", "0.1.0").is_err());
        // The signature check alone passes, so it is the api rule that refuses it.
        update::verify_with(KEY, &manifest_message(api2.as_bytes()), SIG_API2).unwrap();
    }

    #[test]
    fn file_hash_mismatch_is_rejected() {
        let manifest = verify(MANIFEST, SIG, "0.1.0", "v0.2.0", "0.1.0").unwrap();
        assert!(verify_file(&manifest, "ratscan_wasm.js", b"hellp").is_err());
        assert!(verify_file(&manifest, "ratscan_wasm_bg.wasm", b"hello").is_err());
        assert!(verify_file(&manifest, "other.js", b"hello").is_err());
        verify_file(&manifest, "ratscan_wasm_bg.wasm", b"test").unwrap();
    }

    #[test]
    fn asset_urls_are_under_the_plugin_release_downloads() {
        assert_eq!(
            asset_url(&PLUGIN, "v0.2.0", "ratscan_wasm.js"),
            "https://github.com/hhhhhffd/ratka_checker/releases/download/v0.2.0/ratscan_wasm.js"
        );
        let other = KnownPlugin { id: "packconv", repo: "hhhhhffd/conventor-mc-rp" };
        assert!(asset_url(&other, "v0.1.0", "plugin.json").starts_with("https://github.com/hhhhhffd/conventor-mc-rp/releases/download/"));
    }

    #[test]
    fn manifest_with_path_traversal_names_is_rejected() {
        assert!(!valid_file_name("../x.js"));
        assert!(!valid_file_name("a/b.js"));
        assert!(!valid_file_name(".hidden"));
        assert!(!valid_file_name(""));
        assert!(valid_file_name("ratscan_wasm_bg.wasm"));
    }

    fn installed_fixture(paths: &Paths, version: &str) {
        // The signed manifest says 0.2.0, so only that version can be installed with this signature.
        assert_eq!(version, "0.2.0");
        let manifest = verify(MANIFEST, SIG, "0.1.0", "v0.2.0", "0.1.0").unwrap();
        let files = vec![("ratscan_wasm.js".to_owned(), b"hello".to_vec()), ("ratscan_wasm_bg.wasm".to_owned(), b"test".to_vec())];
        install(paths, &PLUGIN, &manifest, MANIFEST.as_bytes(), SIG, &files).unwrap();
    }

    #[test]
    fn install_loads_back_and_detects_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        installed_fixture(&paths, "0.2.0");
        assert_eq!(active_version(&paths, &PLUGIN).unwrap().as_deref(), Some("0.2.0"));
        let installed = load_installed_with(KEY, &paths, &PLUGIN, "0.2.0", "0.1.0").unwrap();
        assert_eq!(installed.read_file("ratscan_wasm.js").unwrap(), b"hello");
        // The embedded key does not know this release.
        assert!(load_installed_with(PUBLIC_KEY, &paths, &PLUGIN, "0.2.0", "0.1.0").is_err());
        std::fs::write(dir.path().join("plugins/ratscan/0.2.0/ratscan_wasm.js"), b"evil").unwrap();
        assert!(load_installed_with(KEY, &paths, &PLUGIN, "0.2.0", "0.1.0").is_err());
        assert!(installed.read_file("ratscan_wasm.js").is_err());
    }

    #[test]
    fn resolve_falls_back_to_the_bundled_copy() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let none = resolve(&paths, &PLUGIN, "0.1.0", "0.1.0");
        assert_eq!((none.source, none.version.as_str(), none.problem.as_deref()), (Source::Bundled, "0.1.0", None));
        // Installed with the test key: the embedded key rejects it, so the baseline runs and the problem is reported.
        installed_fixture(&paths, "0.2.0");
        let rejected = resolve(&paths, &PLUGIN, "0.1.0", "0.1.0");
        assert_eq!(rejected.source, Source::Bundled);
        assert!(rejected.problem.is_some());
    }

    #[test]
    fn prune_keeps_the_active_and_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["0.1.0", "0.2.0", "0.3.0", ".staging-0.4.0"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("active"), "0.3.0").unwrap();
        prune(dir.path(), "0.3.0", Some("0.2.0"));
        let mut left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["0.2.0", "0.3.0", "active"]);
    }

    #[test]
    fn check_is_due_once_a_day_or_an_hour_after_a_delay() {
        let now = Utc::now();
        let ago = |minutes, delayed, error: Option<&str>| PluginRecord {
            last_check: Some(now - Duration::minutes(minutes)),
            error: error.map(str::to_owned),
            delayed,
        };
        assert!(check_due(None, now));
        assert!(check_due(Some(&PluginRecord::default()), now));
        assert!(!check_due(Some(&ago(60, false, None)), now));
        assert!(!check_due(Some(&ago(23 * 60, false, None)), now));
        assert!(check_due(Some(&ago(25 * 60, false, None)), now));
        // A rate limit or a network error is retried after an hour, not a day.
        assert!(!check_due(Some(&ago(30, true, None)), now));
        assert!(check_due(Some(&ago(61, true, None)), now));
        // A real failure keeps the daily pace.
        assert!(!check_due(Some(&ago(61, false, Some("x"))), now));
    }

    #[test]
    fn states_round_trip_and_survive_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        assert!(load_states(&paths).is_empty());
        let mut states = PluginStates::new();
        states.insert("ratscan".into(), PluginRecord { last_check: Some(Utc::now()), error: Some("x".into()), delayed: true });
        save_states(&paths, &states).unwrap();
        assert_eq!(load_states(&paths), states);
        std::fs::write(dir.path().join("plugins/state.json"), "{").unwrap();
        assert!(load_states(&paths).is_empty());
    }

    /// Live check against the real plugin repositories; run by hand:
    /// `cargo test -p k0t1k-core live_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_releases_are_found_and_verify() {
        let client = update::http_client();
        for plugin in [PLUGIN, KnownPlugin { id: "packconv", repo: "hhhhhffd/conventor-mc-rp" }] {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path());
            // From 0.0.0 the release is newer: downloaded, signature and file hashes verified, installed.
            let installed = update(&client, &paths, &plugin, "0.1.1", "0.0.0").await.unwrap();
            println!("{}: installed {installed:?}", plugin.id);
            assert_eq!(installed.as_deref(), Some("0.1.0"));
            let resolved = resolve(&paths, &plugin, "0.1.1", "0.0.0");
            println!("{}: runs {} from {:?}, problem {:?}", plugin.id, resolved.version, resolved.source, resolved.problem);
            assert_eq!((resolved.source, resolved.problem), (Source::Installed, None));
            // Against the bundled 0.1.0 there is nothing newer.
            let again = update(&client, &paths, &plugin, "0.1.1", "0.1.0").await.unwrap();
            println!("{}: with bundled 0.1.0 -> {again:?}", plugin.id);
            assert_eq!(again, None);
        }
    }
}
