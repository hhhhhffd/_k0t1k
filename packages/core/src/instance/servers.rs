//! The multiplayer server list (`<game dir>/servers.dat`, uncompressed NBT) and the Server List Ping.
//! Editing works on single entries and writes every tag the game stored back untouched, so the cached
//! favicon, `hidden` and tags of newer versions survive. The game rewrites the file when it exits, so
//! callers must refuse to edit while the instance is running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::Instance;
use super::links::SharedFolder;
use super::nbt::{self, Compound, Tag};
use crate::error::{Error, IoContext, Result};
use crate::paths::Paths;

const SERVERS_FILE: &str = "servers.dat";
const DEFAULT_PORT: u16 = 25565;
const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// A status response is a few hundred bytes plus an optional 64x64 favicon; this only stops a
/// server from making the launcher buffer an unbounded amount.
const MAX_RESPONSE: usize = 1024 * 1024;

/// Whether the server's resource pack is applied without asking (`acceptTextures` of `servers.dat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourcePackPolicy {
    /// The tag is absent: the game asks on the first connection.
    Prompt,
    Accept,
    Decline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub name: String,
    pub address: String,
    pub resource_pack: ResourcePackPolicy,
}

fn read_root(game_dir: &Path) -> Result<Compound> {
    let path = game_dir.join(SERVERS_FILE);
    match std::fs::metadata(&path) {
        Ok(_) => Ok(nbt::read_file(&path)?.1),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Compound::default()),
        Err(error) => Err(error).at(&path),
    }
}

fn write_root(game_dir: &Path, root: &Compound) -> Result<()> {
    std::fs::create_dir_all(game_dir).at(game_dir)?;
    nbt::write_file(&game_dir.join(SERVERS_FILE), "", root, false)
}

/// The compounds of the `servers` list; a missing list is an empty one.
fn entries_mut(root: &mut Compound) -> Result<&mut Vec<Tag>> {
    if root.get("servers").is_none() {
        root.set("servers", Tag::List(10, Vec::new()));
    }
    match root.get_mut("servers") {
        Some(Tag::List(element, items)) => {
            // An empty list is stored with element type End by some writers.
            if items.is_empty() {
                *element = 10;
            }
            if *element == 10 {
                Ok(items)
            } else {
                Err(Error::Other("servers.dat: `servers` is not a list of compounds".into()))
            }
        }
        _ => Err(Error::Other("servers.dat: `servers` is not a list".into())),
    }
}

fn parse_entry(tag: &Tag) -> Server {
    let compound = match tag {
        Tag::Compound(compound) => compound,
        _ => &Compound::default(),
    };
    Server {
        name: compound.string("name").unwrap_or_default().to_owned(),
        address: compound.string("ip").unwrap_or_default().to_owned(),
        resource_pack: match compound.integer("acceptTextures") {
            Some(0) => ResourcePackPolicy::Decline,
            Some(_) => ResourcePackPolicy::Accept,
            None => ResourcePackPolicy::Prompt,
        },
    }
}

fn apply(compound: &mut Compound, server: &Server) {
    compound.set("name", Tag::String(server.name.clone()));
    compound.set("ip", Tag::String(server.address.clone()));
    match server.resource_pack {
        ResourcePackPolicy::Prompt => compound.0.retain(|(key, _)| key != "acceptTextures"),
        ResourcePackPolicy::Accept => compound.set("acceptTextures", Tag::Byte(1)),
        ResourcePackPolicy::Decline => compound.set("acceptTextures", Tag::Byte(0)),
    }
}

fn validate(server: &Server) -> Result<()> {
    if server.address.trim().is_empty() {
        return Err(Error::Other("the server address is empty".into()));
    }
    Ok(())
}

fn index_error(index: usize, len: usize) -> Error {
    Error::Other(format!("server {index} does not exist (the list has {len})"))
}

pub fn list(game_dir: &Path) -> Result<Vec<Server>> {
    let mut root = read_root(game_dir)?;
    Ok(entries_mut(&mut root)?.iter().map(parse_entry).collect())
}

pub fn add(game_dir: &Path, server: &Server) -> Result<()> {
    validate(server)?;
    let mut root = read_root(game_dir)?;
    let mut compound = Compound::default();
    apply(&mut compound, server);
    entries_mut(&mut root)?.push(Tag::Compound(compound));
    write_root(game_dir, &root)
}

pub fn update(game_dir: &Path, index: usize, server: &Server) -> Result<()> {
    validate(server)?;
    let mut root = read_root(game_dir)?;
    let entries = entries_mut(&mut root)?;
    let len = entries.len();
    match entries.get_mut(index) {
        Some(Tag::Compound(compound)) => apply(compound, server),
        _ => return Err(index_error(index, len)),
    }
    write_root(game_dir, &root)
}

pub fn remove(game_dir: &Path, index: usize) -> Result<()> {
    let mut root = read_root(game_dir)?;
    let entries = entries_mut(&mut root)?;
    if index >= entries.len() {
        return Err(index_error(index, entries.len()));
    }
    entries.remove(index);
    write_root(game_dir, &root)
}

/// Moves the server at `from` so that it ends up at position `to`.
pub fn reorder(game_dir: &Path, from: usize, to: usize) -> Result<()> {
    let mut root = read_root(game_dir)?;
    let entries = entries_mut(&mut root)?;
    let len = entries.len();
    if from >= len || to >= len {
        return Err(index_error(from.max(to), len));
    }
    let entry = entries.remove(from);
    entries.insert(to, entry);
    write_root(game_dir, &root)
}

// ---- Shared list ----
//
// `shared/servers.dat` is a copy kept in sync with the game dirs of the instances that share the list.
// It cannot be a link: the game saves by writing a temporary file and renaming it over `servers.dat`,
// which breaks hardlinks and replaces symlinks. So the shared file is copied into the game dir before
// each launch (`pull_shared`) and merged back after the game exits (`push_shared`). Sessions that
// overlap are resolved on exit: the session that exits last keeps its own order and deletions, and
// entries another session added to the shared file meanwhile are appended, so none are lost.

/// Appends the entries of `extra` whose address (case-insensitive) is not in `base` yet.
fn merge_missing(base: &mut Compound, extra: &mut Compound) -> Result<()> {
    let key = |tag: &Tag| parse_entry(tag).address.trim().to_lowercase();
    let mut known: std::collections::HashSet<String> = entries_mut(base)?.iter().map(key).collect();
    let extra = std::mem::take(entries_mut(extra)?);
    let entries = entries_mut(base)?;
    for tag in extra {
        if known.insert(key(&tag)) {
            entries.push(tag);
        }
    }
    Ok(())
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).at(path),
    }
}

/// Starts sharing the list: the entries of the game dir that the shared file lacks are appended to it
/// (keeping order), then the game dir gets the shared file. Calling it again is harmless.
pub fn enable_shared_list(paths: &Paths, game_dir: &Path) -> Result<()> {
    let shared_dir = paths.shared_dir();
    let mut shared = read_root(&shared_dir)?;
    let mut local = read_root(game_dir)?;
    merge_missing(&mut shared, &mut local)?;
    write_root(&shared_dir, &shared)?;
    write_root(game_dir, &shared)
}

/// The directory whose `servers.dat` is the list of the instance: the shared one when the instance
/// shares its list, the game dir otherwise.
pub fn list_dir(paths: &Paths, instance: &Instance) -> PathBuf {
    if instance.shared_folders.contains(&SharedFolder::Servers) {
        paths.shared_dir()
    } else {
        instance.game_dir(paths)
    }
}

/// Runs an edit on the instance's list and, for a shared list, copies the result into the game dir.
pub fn edit_list<T>(paths: &Paths, instance: &Instance, edit: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let result = edit(&list_dir(paths, instance))?;
    if instance.shared_folders.contains(&SharedFolder::Servers) {
        pull_shared(paths, &instance.game_dir(paths))?;
    }
    Ok(result)
}

/// Copies the shared file over the game dir's `servers.dat` (before a launch). Returns what the game
/// dir holds afterwards (`None`: no file), to be given to [`push_shared`] when the game exits.
pub fn pull_shared(paths: &Paths, game_dir: &Path) -> Result<Option<Vec<u8>>> {
    let local = game_dir.join(SERVERS_FILE);
    let Some(shared) = read_bytes(&paths.shared_dir().join(SERVERS_FILE))? else {
        return read_bytes(&local);
    };
    std::fs::create_dir_all(game_dir).at(game_dir)?;
    std::fs::write(&local, &shared).at(&local)?;
    Ok(Some(shared))
}

/// Stores the game dir's list in the shared file when the game changed it since [`pull_shared`]
/// returned `snapshot`. If the shared file changed meanwhile too, the shared-only entries are appended.
pub fn push_shared(paths: &Paths, game_dir: &Path, snapshot: Option<&[u8]>) -> Result<()> {
    let Some(local) = read_bytes(&game_dir.join(SERVERS_FILE))? else {
        return Ok(());
    };
    if Some(local.as_slice()) == snapshot {
        return Ok(());
    }
    let shared_dir = paths.shared_dir();
    match read_bytes(&shared_dir.join(SERVERS_FILE))? {
        Some(current) if Some(current.as_slice()) != snapshot => {
            let corrupt = |error: Error| Error::Other(format!("servers.dat: {error}"));
            let mut root = nbt::parse(&local).map_err(corrupt)?.1;
            let mut theirs = nbt::parse(&current).map_err(corrupt)?.1;
            merge_missing(&mut root, &mut theirs)?;
            write_root(&shared_dir, &root)
        }
        _ => {
            std::fs::create_dir_all(&shared_dir).at(&shared_dir)?;
            let target = shared_dir.join(SERVERS_FILE);
            std::fs::write(&target, local).at(&target)
        }
    }
}

/// What a server answered to the status request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// Message of the day as plain text (formatting codes removed).
    pub motd: String,
    pub version: String,
    pub protocol: i64,
    pub online: u32,
    pub max: u32,
    /// Round trip of the ping/pong exchange (of the status exchange when the server skips pong).
    pub latency_ms: u64,
    /// `data:image/png;base64,...` as the server sent it.
    pub favicon: Option<String>,
}

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

async fn read_varint(stream: &mut (impl AsyncReadExt + Unpin)) -> Result<i32> {
    let mut value = 0_u32;
    for position in 0..5 {
        let byte = stream.read_u8().await.map_err(ping_io)?;
        value |= u32::from(byte & 0x7F) << (7 * position);
        if byte & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(Error::Other("the server sent a malformed length".into()))
}

fn ping_io(error: std::io::Error) -> Error {
    Error::Other(format!("connection failed: {error}"))
}

fn packet(id: i32, body: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    write_varint(&mut content, id);
    content.extend_from_slice(body);
    let mut out = Vec::new();
    write_varint(&mut out, i32::try_from(content.len()).expect("packets built here are tiny"));
    out.extend_from_slice(&content);
    out
}

/// Splits `host`, `host:port`, `[v6]` and `[v6]:port`; the port defaults to 25565.
fn split_address(address: &str) -> Result<(String, u16)> {
    let address = address.trim();
    let bad_port = |port: &str| Error::Other(format!("{port:?} is not a valid port"));
    let (host, port) = if let Some(rest) = address.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| Error::Other(format!("{address:?} is not a valid address")))?;
        (host, after.strip_prefix(':'))
    } else if address.matches(':').count() == 1 {
        let (host, port) = address.split_once(':').expect("one colon was counted");
        (host, Some(port))
    } else {
        (address, None)
    };
    if host.is_empty() {
        return Err(Error::Other("the server address is empty".into()));
    }
    let port = match port {
        Some(port) => port.parse().map_err(|_| bad_port(port))?,
        None => DEFAULT_PORT,
    };
    Ok((host.to_owned(), port))
}

fn handshake(host: &str, port: u16) -> Vec<u8> {
    let mut body = Vec::new();
    // Protocol -1: "unknown", accepted by servers for status requests.
    write_varint(&mut body, -1);
    write_varint(&mut body, i32::try_from(host.len()).expect("a host name is short"));
    body.extend_from_slice(host.as_bytes());
    body.extend_from_slice(&port.to_be_bytes());
    // Next state: status.
    write_varint(&mut body, 1);
    packet(0, &body)
}

/// Plain text of a chat component (`description`): a string, or an object with `text` and `extra`.
fn chat_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
            if let Some(Value::Array(extra)) = object.get("extra") {
                for part in extra {
                    chat_text(part, out);
                }
            }
        }
        Value::Array(parts) => {
            for part in parts {
                chat_text(part, out);
            }
        }
        _ => {}
    }
}

/// Removes `§x` formatting codes.
fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_status(json: &str, latency: Duration) -> Result<ServerStatus> {
    let value: Value = serde_json::from_str(json).map_err(|error| Error::Other(format!("the server sent an invalid status: {error}")))?;
    let mut motd = String::new();
    if let Some(description) = value.get("description") {
        chat_text(description, &mut motd);
    }
    let number = |path: &[&str]| path.iter().try_fold(&value, |value, key| value.get(*key)).and_then(Value::as_i64);
    Ok(ServerStatus {
        motd: strip_formatting(&motd),
        version: value.pointer("/version/name").and_then(Value::as_str).map(strip_formatting).unwrap_or_default(),
        protocol: number(&["version", "protocol"]).unwrap_or(-1),
        online: number(&["players", "online"]).map_or(0, |n| u32::try_from(n).unwrap_or(0)),
        max: number(&["players", "max"]).map_or(0, |n| u32::try_from(n).unwrap_or(0)),
        latency_ms: u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
        favicon: value.get("favicon").and_then(Value::as_str).filter(|icon| icon.starts_with("data:image/png;base64,")).map(str::to_owned),
    })
}

async fn ping_exchange(host: &str, port: u16) -> Result<ServerStatus> {
    let mut stream = TcpStream::connect((host, port)).await.map_err(ping_io)?;
    let started = Instant::now();
    stream.write_all(&handshake(host, port)).await.map_err(ping_io)?;
    stream.write_all(&packet(0, &[])).await.map_err(ping_io)?;

    let length = read_varint(&mut stream).await?;
    let length = usize::try_from(length).ok().filter(|length| (1..=MAX_RESPONSE).contains(length));
    let length = length.ok_or_else(|| Error::Other("the server sent a malformed status response".into()))?;
    let mut response = vec![0; length];
    stream.read_exact(&mut response).await.map_err(ping_io)?;
    let status_latency = started.elapsed();

    let mut reader = response.as_slice();
    if read_varint(&mut reader).await? != 0 {
        return Err(Error::Other("the server answered with an unexpected packet".into()));
    }
    let json_len = usize::try_from(read_varint(&mut reader).await?).map_err(|_| Error::Other("malformed status response".into()))?;
    if json_len > reader.len() {
        return Err(Error::Other("malformed status response".into()));
    }
    let json = std::str::from_utf8(&reader[..json_len]).map_err(|_| Error::Other("the status is not valid UTF-8".into()))?;
    let mut status = parse_status(json, status_latency)?;

    // The pong measures the network better than the status exchange, which includes the server's
    // own work; a server that does not answer it still gets a latency.
    let payload = 0x6B30_746B_i64;
    let pong_started = Instant::now();
    if stream.write_all(&packet(1, &payload.to_be_bytes())).await.is_ok()
        && let Ok(length) = read_varint(&mut stream).await
        && (1..=64).contains(&length)
    {
        let mut pong = vec![0; length as usize];
        if stream.read_exact(&mut pong).await.is_ok() {
            status.latency_ms = u64::try_from(pong_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        }
    }
    Ok(status)
}

/// Asks the server for its status (Server List Ping). `address` is `host` or `host:port`. SRV records
/// (`_minecraft._tcp`) are not resolved: a server that relies on one is pinged on the default port.
pub async fn ping(address: &str) -> Result<ServerStatus> {
    let (host, port) = split_address(address)?;
    tokio::time::timeout(PING_TIMEOUT, ping_exchange(&host, port))
        .await
        .map_err(|_| Error::Other(format!("{address} did not answer within {} s", PING_TIMEOUT.as_secs())))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_setup() -> (tempfile::TempDir, Paths, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"));
        let game = dir.path().join("data/instances/a/minecraft");
        std::fs::create_dir_all(&game).unwrap();
        (dir, paths, game)
    }

    fn addresses(dir: &Path) -> Vec<String> {
        list(dir).unwrap().into_iter().map(|server| server.address).collect()
    }

    fn put(dir: &Path, address: &str) {
        add(dir, &server(address, address, ResourcePackPolicy::Prompt)).unwrap();
    }

    #[test]
    fn enabling_merges_new_addresses_in_order_and_copies_back() {
        let (_dir, paths, game) = shared_setup();
        put(&paths.shared_dir(), "one.example");
        put(&paths.shared_dir(), "two.example");
        put(&game, "TWO.example");
        put(&game, "mine.example");

        enable_shared_list(&paths, &game).unwrap();
        enable_shared_list(&paths, &game).unwrap();

        let expected = ["one.example", "two.example", "mine.example"];
        assert_eq!(addresses(&paths.shared_dir()), expected);
        assert_eq!(addresses(&game), expected);
    }

    #[test]
    fn pull_then_push_stores_only_game_changes() {
        let (_dir, paths, game) = shared_setup();
        put(&paths.shared_dir(), "one.example");

        let snapshot = pull_shared(&paths, &game).unwrap();
        assert_eq!(addresses(&game), ["one.example"]);
        push_shared(&paths, &game, snapshot.as_deref()).unwrap();
        assert_eq!(std::fs::read(paths.shared_dir().join("servers.dat")).unwrap(), snapshot.clone().unwrap());

        // The game removes a server and adds another: the instance wins over the unchanged shared file.
        remove(&game, 0).unwrap();
        put(&game, "new.example");
        push_shared(&paths, &game, snapshot.as_deref()).unwrap();
        assert_eq!(addresses(&paths.shared_dir()), ["new.example"]);
    }

    #[test]
    fn push_keeps_entries_another_session_added() {
        let (_dir, paths, game) = shared_setup();
        put(&paths.shared_dir(), "one.example");
        let snapshot = pull_shared(&paths, &game).unwrap();

        put(&game, "mine.example");
        // Another session exits first and adds its own server (and the same one this session added).
        put(&paths.shared_dir(), "other.example");
        put(&paths.shared_dir(), "mine.example");
        push_shared(&paths, &game, snapshot.as_deref()).unwrap();

        assert_eq!(addresses(&paths.shared_dir()), ["one.example", "mine.example", "other.example"]);
    }

    #[test]
    fn pull_without_shared_file_keeps_the_game_copy_and_edits_follow_the_shared_one() {
        let (_dir, paths, game) = shared_setup();
        put(&game, "local.example");
        let snapshot = pull_shared(&paths, &game).unwrap();
        assert_eq!(addresses(&game), ["local.example"]);
        push_shared(&paths, &game, snapshot.as_deref()).unwrap();
        assert!(!paths.shared_dir().join("servers.dat").exists());

        let mut instance = crate::instance::Instance::from_json(
            br#"{"name":"A","components":[],"created":"2026-01-01T00:00:00Z","sharedFolders":["servers"]}"#,
        )
        .unwrap();
        instance.id = "a".into();
        enable_shared_list(&paths, &game).unwrap();
        edit_list(&paths, &instance, |dir| add(dir, &server("n", "added.example", ResourcePackPolicy::Prompt))).unwrap();
        assert_eq!(addresses(&paths.shared_dir()), ["local.example", "added.example"]);
        assert_eq!(addresses(&game), ["local.example", "added.example"]);
        assert_eq!(list_dir(&paths, &instance), paths.shared_dir());
    }

    fn server(name: &str, address: &str, resource_pack: ResourcePackPolicy) -> Server {
        Server { name: name.into(), address: address.into(), resource_pack }
    }

    #[test]
    fn servers_round_trip_and_unknown_tags_survive() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).unwrap().is_empty());

        add(dir.path(), &server("Home 🌍", "mc.example.com:25570", ResourcePackPolicy::Accept)).unwrap();
        add(dir.path(), &server("B", "b.example", ResourcePackPolicy::Prompt)).unwrap();
        add(dir.path(), &server("C", "c.example", ResourcePackPolicy::Decline)).unwrap();

        // The game stores a cached favicon and other tags; editing must not drop them.
        let path = dir.path().join("servers.dat");
        let (_, mut root) = nbt::read_file(&path).unwrap();
        let Some(Tag::List(_, entries)) = root.get_mut("servers") else { panic!("no list") };
        let Some(Tag::Compound(first)) = entries.first_mut() else { panic!("no entry") };
        first.set("icon", Tag::String("AAAA".into()));
        first.set("hidden", Tag::Byte(1));
        nbt::write_file(&path, "", &root, false).unwrap();

        update(dir.path(), 0, &server("Renamed", "new.example", ResourcePackPolicy::Prompt)).unwrap();
        let listed = list(dir.path()).unwrap();
        assert_eq!(listed[0], server("Renamed", "new.example", ResourcePackPolicy::Prompt));
        assert_eq!(listed[1].resource_pack, ResourcePackPolicy::Prompt);
        assert_eq!(listed[2].resource_pack, ResourcePackPolicy::Decline);

        let (_, root) = nbt::read_file(&path).unwrap();
        let Some(Tag::List(_, entries)) = root.get("servers") else { panic!("no list") };
        let Tag::Compound(first) = &entries[0] else { panic!("no entry") };
        assert_eq!(first.string("icon"), Some("AAAA"));
        assert_eq!(first.integer("hidden"), Some(1));
        assert_eq!(first.get("acceptTextures"), None);

        reorder(dir.path(), 2, 0).unwrap();
        let names: Vec<_> = list(dir.path()).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["C", "Renamed", "B"]);
        remove(dir.path(), 1).unwrap();
        let names: Vec<_> = list(dir.path()).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["C", "B"]);
    }

    #[test]
    fn bad_indexes_and_addresses_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), &server("A", "a", ResourcePackPolicy::Prompt)).unwrap();
        assert!(update(dir.path(), 1, &server("A", "a", ResourcePackPolicy::Prompt)).is_err());
        assert!(remove(dir.path(), 5).is_err());
        assert!(reorder(dir.path(), 0, 1).is_err());
        assert!(add(dir.path(), &server("A", "  ", ResourcePackPolicy::Prompt)).is_err());
        assert_eq!(list(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn addresses_are_split_with_the_default_port() {
        assert_eq!(split_address("mc.example.com").unwrap(), ("mc.example.com".into(), 25565));
        assert_eq!(split_address(" mc.example.com:1234 ").unwrap(), ("mc.example.com".into(), 1234));
        assert_eq!(split_address("[::1]:99").unwrap(), ("::1".into(), 99));
        assert_eq!(split_address("[::1]").unwrap(), ("::1".into(), 25565));
        assert_eq!(split_address("::1").unwrap(), ("::1".into(), 25565));
        assert!(split_address("host:abc").is_err());
        assert!(split_address(":25565").is_err());
        assert!(split_address("").is_err());
    }

    #[test]
    fn status_json_is_parsed_in_all_description_forms() {
        let modern = r#"{"version":{"name":"Paper 1.21.1","protocol":767},"players":{"max":100,"online":7},
            "description":{"text":"Hello ","extra":[{"text":"§aworld","color":"red"},{"text":"!"}]},
            "favicon":"data:image/png;base64,AAAA"}"#;
        let status = parse_status(modern, Duration::from_millis(42)).unwrap();
        assert_eq!(
            status,
            ServerStatus {
                motd: "Hello world!".into(),
                version: "Paper 1.21.1".into(),
                protocol: 767,
                online: 7,
                max: 100,
                latency_ms: 42,
                favicon: Some("data:image/png;base64,AAAA".into()),
            }
        );

        let legacy = parse_status(r#"{"description":"§6Gold §lbold","players":{"online":-1,"max":5}}"#, Duration::ZERO).unwrap();
        assert_eq!(legacy.motd, "Gold bold");
        assert_eq!((legacy.online, legacy.max, legacy.protocol, legacy.favicon), (0, 5, -1, None));

        assert!(parse_status("not json", Duration::ZERO).is_err());
    }

    #[test]
    fn varints_use_the_protocol_encoding() {
        let encode = |value| {
            let mut out = Vec::new();
            write_varint(&mut out, value);
            out
        };
        assert_eq!(encode(0), [0]);
        assert_eq!(encode(300), [0xAC, 0x02]);
        assert_eq!(encode(-1), [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        assert_eq!(handshake("a", 25565)[..3], [11, 0, 0xFF]);
    }

    #[tokio::test]
    async fn ping_talks_to_a_status_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Handshake, then the status request.
            for expected_id in [0, 0] {
                let length = read_varint(&mut stream).await.unwrap() as usize;
                let mut data = vec![0; length];
                stream.read_exact(&mut data).await.unwrap();
                assert_eq!(data[0], expected_id);
            }
            let json = br#"{"version":{"name":"1.21.1","protocol":767},"players":{"max":20,"online":3},"description":"Test"}"#;
            let mut body = Vec::new();
            write_varint(&mut body, i32::try_from(json.len()).unwrap());
            body.extend_from_slice(json);
            stream.write_all(&packet(0, &body)).await.unwrap();
            // Pong echoes the ping payload.
            let length = read_varint(&mut stream).await.unwrap() as usize;
            let mut data = vec![0; length];
            stream.read_exact(&mut data).await.unwrap();
            stream.write_all(&packet(1, &data[1..])).await.unwrap();
        });

        let status = ping(&format!("127.0.0.1:{port}")).await.unwrap();
        assert_eq!((status.motd.as_str(), status.online, status.max), ("Test", 3, 20));
        assert_eq!(status.version, "1.21.1");
    }

    #[tokio::test]
    async fn ping_reports_a_refused_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = ping(&format!("127.0.0.1:{port}")).await.unwrap_err().to_string();
        assert!(error.contains("connection failed"), "{error}");
    }
}
