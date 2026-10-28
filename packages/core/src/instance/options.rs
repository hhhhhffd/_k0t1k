//! The game's settings file, `<game dir>/options.txt`: one `key:value` line per option. Editing
//! replaces the value of a line in place and appends options the file does not have yet, so the order,
//! unknown lines and the options of other versions stay as the game wrote them. The game rewrites the
//! file when it exits, so callers must refuse to edit while the instance is running.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};

const OPTIONS_FILE: &str = "options.txt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameOption {
    pub key: String,
    /// The raw value as stored: strings keep their quotes, lists their brackets.
    pub value: String,
}

/// The options in file order; `None` while the game has not created the file yet.
pub fn read(game_dir: &Path) -> Result<Option<Vec<GameOption>>> {
    let path = game_dir.join(OPTIONS_FILE);
    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).at(&path),
    };
    Ok(Some(text.lines().filter_map(parse_line).collect()))
}

fn parse_line(line: &str) -> Option<GameOption> {
    let (key, value) = line.trim_end_matches('\r').split_once(':')?;
    Some(GameOption { key: key.to_owned(), value: value.to_owned() })
}

/// Sets the given options, creating the file when it is missing. Keys may not contain `:` and values
/// may not span lines. The file is replaced atomically.
pub fn write(game_dir: &Path, changes: &[GameOption]) -> Result<()> {
    for change in changes {
        if change.key.is_empty() || change.key.contains([':', '\n', '\r']) || change.value.contains(['\n', '\r']) {
            return Err(Error::Other(format!("invalid option {:?}", change.key)));
        }
    }
    let path = game_dir.join(OPTIONS_FILE);
    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).at(&path),
    };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    for change in changes {
        let line = format!("{}:{}", change.key, change.value);
        match lines.iter_mut().find(|l| parse_line(l).is_some_and(|o| o.key == change.key)) {
            Some(existing) => *existing = line,
            None => lines.push(line),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    let temp = game_dir.join(format!("{OPTIONS_FILE}.k0t1k-tmp"));
    std::fs::create_dir_all(game_dir).at(game_dir)?;
    std::fs::write(&temp, out).at(&temp)?;
    std::fs::rename(&temp, &path).at(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(key: &str, value: &str) -> GameOption {
        GameOption { key: key.to_owned(), value: value.to_owned() }
    }

    #[test]
    fn missing_file_reads_as_none_and_is_created() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap(), None);
        write(dir.path(), &[option("fov", "0.5")]).unwrap();
        assert_eq!(read(dir.path()).unwrap(), Some(vec![option("fov", "0.5")]));
    }

    #[test]
    fn edits_in_place_and_appends_new_keys() {
        let dir = tempfile::tempdir().unwrap();
        let original = "version:4671\nkey_key.forward:key.keyboard.w\nresourcePacks:[\"vanilla\",\"file/a:b\"]\ngarbage line\nfov:0.0\n";
        std::fs::write(dir.path().join(OPTIONS_FILE), original).unwrap();
        write(dir.path(), &[option("fov", "0.5"), option("renderDistance", "8")]).unwrap();
        let text = std::fs::read_to_string(dir.path().join(OPTIONS_FILE)).unwrap();
        assert_eq!(
            text,
            "version:4671\nkey_key.forward:key.keyboard.w\nresourcePacks:[\"vanilla\",\"file/a:b\"]\ngarbage line\nfov:0.5\nrenderDistance:8\n"
        );
        let options = read(dir.path()).unwrap().unwrap();
        assert_eq!(options[2], option("resourcePacks", "[\"vanilla\",\"file/a:b\"]"));
        assert_eq!(options.len(), 5);
    }

    #[test]
    fn rejects_keys_and_values_that_would_break_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write(dir.path(), &[option("a:b", "1")]).is_err());
        assert!(write(dir.path(), &[option("a", "1\nb:2")]).is_err());
        assert!(write(dir.path(), &[option("", "1")]).is_err());
        assert_eq!(read(dir.path()).unwrap(), None);
    }
}
