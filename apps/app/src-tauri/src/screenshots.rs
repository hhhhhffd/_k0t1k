//! Screenshots of an instance (`screenshots/`, possibly a link to the shared folder). The UI shows them
//! through the asset protocol (`convertFileSrc`), whose scope is widened to the folder when it is listed.

use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use k0t1k_core::error::{Error, IoContext};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{CommandResult, blocking};
use crate::state::AppState;
use crate::worlds::game_dir;

const FOLDER: &str = "screenshots";
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
    file_name: String,
    /// Absolute path, for `convertFileSrc` and for the open command.
    path: String,
    size: u64,
    /// Milliseconds since the Unix epoch.
    modified: u64,
}

fn scan(folder: &Path) -> k0t1k_core::Result<Vec<Screenshot>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at(folder),
    };
    let mut shots = Vec::new();
    for entry in entries {
        let entry = entry.at(folder)?;
        let path = entry.path();
        let is_image = path.extension().is_some_and(|ext| EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)));
        if !is_image || !path.is_file() {
            continue;
        }
        let metadata = entry.metadata().at(&path)?;
        let modified = metadata.modified().at(&path)?.duration_since(UNIX_EPOCH).map_or(0, |age| age.as_millis() as u64);
        shots.push(Screenshot {
            file_name: entry.file_name().to_string_lossy().into_owned(),
            path: path.display().to_string(),
            size: metadata.len(),
            modified,
        });
    }
    shots.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.file_name.cmp(&b.file_name)));
    Ok(shots)
}

fn screenshot_path(folder: PathBuf, file_name: &str) -> k0t1k_core::Result<PathBuf> {
    let mut components = Path::new(file_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(folder.join(file_name)),
        _ => Err(Error::Other(format!("{file_name:?} is not a plain file name"))),
    }
}

#[tauri::command]
pub async fn list_screenshots(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<Vec<Screenshot>> {
    let folder = game_dir(&state, &instance_id).await?.join(FOLDER);
    // Only this instance's screenshots folder becomes readable by the webview, not the data dir.
    app.asset_protocol_scope().allow_directory(&folder, false)?;
    blocking(move || scan(&folder)).await
}

#[tauri::command]
pub async fn open_screenshot(app: AppHandle, state: State<'_, AppState>, instance_id: String, file_name: String) -> CommandResult<()> {
    let folder = game_dir(&state, &instance_id).await?.join(FOLDER);
    let path = blocking(move || screenshot_path(folder, &file_name)).await?;
    app.opener().open_path(path.display().to_string(), None::<&str>)?;
    Ok(())
}

#[tauri::command]
pub async fn delete_screenshot(state: State<'_, AppState>, instance_id: String, file_name: String) -> CommandResult<()> {
    let folder = game_dir(&state, &instance_id).await?.join(FOLDER);
    blocking(move || {
        let path = screenshot_path(folder, &file_name)?;
        std::fs::remove_file(&path).at(&path)
    })
    .await
}

/// Opens the folder, creating it first: the game only creates it with the first screenshot.
#[tauri::command]
pub async fn open_screenshots_folder(app: AppHandle, state: State<'_, AppState>, instance_id: String) -> CommandResult<()> {
    let folder = game_dir(&state, &instance_id).await?.join(FOLDER);
    let target = folder.clone();
    blocking(move || std::fs::create_dir_all(&target).at(&target)).await?;
    app.opener().open_path(folder.display().to_string(), None::<&str>)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_images_newest_first_and_names_must_be_plain() {
        let dir = std::env::temp_dir().join("k0t1k-screenshots-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), [1]).unwrap();
        std::fs::write(dir.join("b.JPG"), [1, 2]).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::create_dir(dir.join("sub.png")).unwrap();

        let shots = scan(&dir).unwrap();
        let mut names: Vec<_> = shots.iter().map(|shot| shot.file_name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["a.png", "b.JPG"]);
        assert!(scan(&dir.join("missing")).unwrap().is_empty());

        assert!(screenshot_path(dir.clone(), "a.png").is_ok());
        assert!(screenshot_path(dir.clone(), "../a.png").is_err());
        assert!(screenshot_path(dir.clone(), "sub/a.png").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
