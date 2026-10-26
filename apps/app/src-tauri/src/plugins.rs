//! Wasm plugins (RatScan, packconv): which copy runs, updates, and the file access the plugin
//! features need. The plugins themselves run in a Web Worker of the UI, which has no Tauri IPC, so
//! everything a plugin sees is passed to it as bytes by these commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use k0t1k_core::component::MINECRAFT_UID;
use k0t1k_core::i18n::tr;
use k0t1k_core::error::IoContext;
use k0t1k_core::content::{self as core_content, ResourceKind};
use k0t1k_core::packformat;
use k0t1k_core::plugins::{self, KnownPlugin, Manifest, PluginRecord, Resolved, Source};
use k0t1k_core::scancache::{self, ScanEntry};
use k0t1k_core::update;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use tauri::ipc::{InvokeBody, Request, Response};

use crate::content::instance_of;
use crate::error::{AppError, CommandResult, blocking};
use crate::state::AppState;

const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// A mod or pack is read whole into memory for the plugin; anything bigger is not a normal jar.
const MAX_READ_BYTES: u64 = 256 * 1024 * 1024;

/// A known plugin and the manifest of the baseline shipped in the frontend assets.
struct Entry {
    plugin: KnownPlugin,
    bundled_manifest: &'static str,
}

const ENTRIES: [Entry; 2] = [
    Entry {
        plugin: KnownPlugin { id: "ratscan", repo: "hhhhhffd/ratka_checker" },
        bundled_manifest: include_str!("../../public/plugins/ratscan/plugin.json"),
    },
    Entry {
        plugin: KnownPlugin { id: "packconv", repo: "hhhhhffd/conventor-mc-rp" },
        bundled_manifest: include_str!("../../public/plugins/packconv/plugin.json"),
    },
];

impl Entry {
    fn bundled(&self) -> Manifest {
        serde_json::from_str(self.bundled_manifest).expect("the bundled plugin.json is valid")
    }
}

fn entry(id: &str) -> CommandResult<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.plugin.id == id).ok_or_else(|| AppError::new(format!("unknown plugin {id:?}")))
}

/// Serialises update checks and the state file.
static CHECK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    id: &'static str,
    version: String,
    source: Source,
    /// File names the worker loads, from the manifest of the running copy.
    files: Vec<String>,
    last_check: Option<DateTime<Utc>>,
    error: Option<String>,
    /// The last check hit a rate limit or a network problem; the UI shows a calm notice, not an error.
    delayed: bool,
}

fn resolve(state: &AppState, entry: &Entry) -> (Resolved, Manifest) {
    let bundled = entry.bundled();
    let resolved = plugins::resolve(&state.paths, &entry.plugin, LAUNCHER_VERSION, &bundled.version);
    if let Some(problem) = &resolved.problem {
        tracing::warn!("the installed plugin {} cannot be used, the bundled copy runs: {problem}", entry.plugin.id);
    }
    let manifest = resolved.installed.as_ref().map_or(bundled, |installed| installed.manifest.clone());
    (resolved, manifest)
}

fn describe(state: &AppState) -> Vec<PluginInfo> {
    let records = plugins::load_states(&state.paths);
    ENTRIES
        .iter()
        .map(|entry| {
            let (resolved, manifest) = resolve(state, entry);
            let record = records.get(entry.plugin.id).cloned().unwrap_or_default();
            PluginInfo {
                id: entry.plugin.id,
                version: resolved.version,
                source: resolved.source,
                files: manifest.files.into_keys().collect(),
                last_check: record.last_check,
                error: resolved
                    .problem
                    .map(|_| {
                        tr(
                            "The downloaded plugin is damaged, the bundled copy is used. Details are in launcher.log.",
                            "Скачанный плагин повреждён, используется встроенный. Подробности в launcher.log.",
                        )
                        .to_owned()
                    })
                    .or(record.error),
                delayed: record.delayed,
            }
        })
        .collect()
}

#[tauri::command]
pub async fn list_plugins(state: State<'_, AppState>) -> CommandResult<Vec<PluginInfo>> {
    let state = state.inner().clone();
    Ok(tokio::task::spawn_blocking(move || describe(&state)).await?)
}

/// Checks the plugins for newer signed releases and installs them; part of the launcher's update check
/// (`update::check_for_update`). Failures are logged and kept per plugin for Settings, a short localized
/// text for a rejected release and a `delayed` flag for a transient problem; the active version never
/// changes on a failure.
pub async fn check_all(state: &AppState) {
    let _guard = CHECK_LOCK.lock().await;
    let client = update::http_client();
    let mut records = plugins::load_states(&state.paths);
    for entry in &ENTRIES {
        let current = resolve(state, entry).0.version;
        let result = plugins::update(&client, &state.paths, &entry.plugin, LAUNCHER_VERSION, &current).await;
        let record: &mut PluginRecord = records.entry(entry.plugin.id.to_owned()).or_default();
        record.last_check = Some(Utc::now());
        match result {
            Ok(Some(version)) => {
                tracing::info!("plugin {} updated to {version}", entry.plugin.id);
                (record.error, record.delayed) = (None, false);
            }
            Ok(None) => (record.error, record.delayed) = (None, false),
            Err(error) => {
                tracing::warn!("plugin {} update failed: {error}", entry.plugin.id);
                record.delayed = update::is_transient(&error);
                record.error = (!record.delayed).then(|| {
                    tr(
                        "The plugin update could not be verified or installed. Details are in launcher.log.",
                        "Не удалось проверить или установить обновление плагина. Подробности в launcher.log.",
                    )
                    .to_owned()
                });
            }
        }
    }
    if let Err(error) = plugins::save_states(&state.paths, &records) {
        tracing::warn!("cannot save the plugin state: {error}");
    }
}

/// A file of the installed copy of a plugin (checked against the signed manifest again). The bundled
/// copy is read by the UI from its own assets.
#[tauri::command]
pub async fn read_plugin_file(state: State<'_, AppState>, id: String, version: String, name: String) -> CommandResult<Response> {
    let entry = entry(&id)?;
    let state = state.inner().clone();
    let bytes = blocking(move || {
        let (resolved, _) = resolve(&state, entry);
        match resolved.installed {
            Some(installed) if resolved.version == version => installed.read_file(&name),
            _ => Err(k0t1k_core::Error::Other(format!("version {version} of {} is not the installed one", entry.plugin.id))),
        }
    })
    .await?;
    Ok(Response::new(bytes))
}

// ---- files for the plugins ----

fn jar_names(game_dir: &std::path::Path) -> std::io::Result<Vec<String>> {
    let folder = game_dir.join(ResourceKind::Mod.folder());
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if entry.file_type()?.is_file() && (lower.ends_with(".jar") || lower.ends_with(".jar.disabled")) {
            names.push(name);
        }
    }
    names.sort_by_key(|name| name.to_lowercase());
    Ok(names)
}

/// The cached result of one jar without the report, for badges.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanBrief {
    verdict: String,
    risk_score: Option<u64>,
    summary: String,
    scanned_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarScan {
    file_name: String,
    enabled: bool,
    size: u64,
    sha1: String,
    /// Present when this jar was scanned by the running version of the plugin.
    scan: Option<ScanBrief>,
}

fn brief(entry: &ScanEntry) -> Option<ScanBrief> {
    let report = entry.result.get("report")?;
    Some(ScanBrief {
        verdict: report.get("verdict")?.as_str()?.to_owned(),
        risk_score: report.get("risk_score").and_then(Value::as_u64),
        summary: entry.result.get("summary").and_then(Value::as_str).unwrap_or_default().to_owned(),
        scanned_at: entry.scanned_at,
    })
}

/// Serialises writes of `ratscan.json` (scans of several jars finish concurrently).
static CACHE_LOCK: Mutex<()> = Mutex::new(());

/// Every jar of the instance (enabled and disabled) with its SHA-1 and its cached scan result.
#[tauri::command]
pub async fn ratscan_status(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<JarScan>> {
    let instance = instance_of(&state, &instance_id).await?;
    let (game_dir, instance_dir) = (instance.game_dir(&state.paths), instance.dir(&state.paths));
    let state = state.inner().clone();
    blocking(move || {
        let version = resolve(&state, entry("ratscan").expect("ratscan is known")).0.version;
        let cache = scancache::load(&instance_dir);
        let mut jars = Vec::new();
        for name in jar_names(&game_dir).map_err(|error| k0t1k_core::Error::Other(error.to_string()))? {
            let path = game_dir.join(ResourceKind::Mod.folder()).join(&name);
            let sha1 = match scancache::sha1_of(&path) {
                Ok(sha1) => sha1,
                // A file that vanished or is locked is skipped; the next status call sees it again.
                Err(error) => {
                    tracing::warn!("cannot hash {}: {error}", path.display());
                    continue;
                }
            };
            let scan = cache.get(&sha1).filter(|entry| entry.plugin_version == version).and_then(brief);
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            jars.push(JarScan { enabled: !name.ends_with(".disabled"), file_name: name, size, sha1, scan });
        }
        Ok(jars)
    })
    .await
}

/// Saves what the plugin returned for the jar with `sha1`.
#[tauri::command]
pub async fn ratscan_store(state: State<'_, AppState>, instance_id: String, sha1: String, plugin_version: String, result: Value) -> CommandResult<()> {
    if sha1.len() != 40 || !sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::new("bad SHA-1"));
    }
    if result.pointer("/report/verdict").and_then(Value::as_str).is_none() {
        return Err(AppError::new("the scan result has no verdict"));
    }
    let instance_dir = instance_of(&state, &instance_id).await?.dir(&state.paths);
    let entry = ScanEntry { plugin_version, scanned_at: Utc::now(), result };
    blocking(move || {
        let _guard = CACHE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        scancache::store(&instance_dir, &sha1.to_ascii_lowercase(), entry)
    })
    .await
}

/// The full `{summary, report}` of the jar with `sha1`, for the details panel.
#[tauri::command]
pub async fn ratscan_report(state: State<'_, AppState>, instance_id: String, sha1: String) -> CommandResult<Option<Value>> {
    let instance_dir = instance_of(&state, &instance_id).await?.dir(&state.paths);
    blocking(move || Ok(scancache::load(&instance_dir).remove(&sha1.to_ascii_lowercase()).map(|entry| entry.result))).await
}

/// The bytes of a mod jar or a resource pack for a plugin (binary IPC, no JSON array of numbers).
#[tauri::command]
pub async fn read_resource_file(state: State<'_, AppState>, instance_id: String, kind: ResourceKind, file_name: String) -> CommandResult<Response> {
    let game_dir = instance_of(&state, &instance_id).await?.game_dir(&state.paths);
    let bytes = blocking(move || core_content::read_resource(&game_dir, kind, &file_name, MAX_READ_BYTES)).await?;
    Ok(Response::new(bytes))
}

/// Arguments of [`save_resource_file`]; they travel in a header because the body is the file.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveArgs {
    instance_id: String,
    kind: ResourceKind,
    file_name: String,
}

/// Saves the raw request body as a new file of the resource folder (never replacing a file) and
/// returns the name used. The `x-args` header is the base64 of the JSON [`SaveArgs`].
#[tauri::command]
pub async fn save_resource_file(state: State<'_, AppState>, request: Request<'_>) -> CommandResult<String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::new("the file must be sent as raw bytes"));
    };
    let header = request.headers().get("x-args").ok_or_else(|| AppError::new("missing arguments"))?;
    let args: SaveArgs = STANDARD
        .decode(header.as_bytes())
        .ok()
        .and_then(|json| serde_json::from_slice(&json).ok())
        .ok_or_else(|| AppError::new("bad arguments"))?;
    let game_dir = instance_of(&state, &args.instance_id).await?.game_dir(&state.paths);
    let bytes = bytes.clone();
    blocking(move || core_content::write_resource_new(&game_dir, args.kind, &args.file_name, &bytes)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackCompat {
    file_name: String,
    /// Releases the pack declares, like `1.20-1.20.1`; `None` when it has no readable `pack.mcmeta`.
    made_for: Option<String>,
    /// The pack's declared formats do not include the instance's Minecraft version.
    mismatch: bool,
}

/// For every zip resource pack of the instance: the releases it was made for and whether that
/// differs from the instance's Minecraft version. Folders and unreadable zips are left out.
#[tauri::command]
pub async fn resourcepack_compat(state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<PackCompat>> {
    let instance = instance_of(&state, &instance_id).await?;
    let game_dir = instance.game_dir(&state.paths);
    let version = instance.components.iter().find(|component| component.uid == MINECRAFT_UID).map(|component| component.version.clone());
    blocking(move || {
        let wanted = version.as_deref().and_then(packformat::format_of_version);
        let folder: PathBuf = game_dir.join(ResourceKind::ResourcePack.folder());
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(k0t1k_core::Error::Other(format!("cannot read {}: {error}", folder.display()))),
        };
        let mut packs = HashMap::new();
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let lower = file_name.to_ascii_lowercase();
            if !(lower.ends_with(".zip") || lower.ends_with(".zip.disabled")) || !entry.path().is_file() {
                continue;
            }
            let range = match std::fs::File::open(entry.path()).at(entry.path()).and_then(packformat::zip_declared_range) {
                Ok(range) => range,
                Err(error) => {
                    tracing::warn!("cannot read the pack format of {}: {error}", entry.path().display());
                    continue;
                }
            };
            let mismatch = matches!((range, wanted), (Some((min, max)), Some(wanted)) if wanted < min || wanted > max);
            packs.insert(file_name.clone(), PackCompat { file_name, made_for: range.map(packformat::releases_label), mismatch });
        }
        Ok(packs.into_values().collect())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_baselines_match_their_manifests() {
        for entry in &ENTRIES {
            let manifest = entry.bundled();
            assert_eq!(manifest.id, entry.plugin.id);
            assert_eq!(manifest.api, plugins::API);
            for name in manifest.files.keys() {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../public/plugins").join(entry.plugin.id).join(name);
                let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                plugins::verify_file(&manifest, name, &bytes).unwrap();
            }
        }
    }
}
