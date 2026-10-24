//! User-provided files in the data directory: instance icons, cat packs, icon sets and theme files.
//! The webview shows them through the asset protocol, so listing a folder also allows reading it.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate};
use k0t1k_core::error::IoContext;
use k0t1k_core::settings::{CustomTheme, UiSettings};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::error::{CommandResult, blocking};
use crate::state::AppState;

const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
const VIDEO_EXTENSIONS: [&str; 2] = ["webm", "mp4"];
const ICON_SET_EXTENSIONS: [&str; 4] = ["png", "svg", "gif", "webp"];
const CAT_PACK_INDEX: &str = "index.json";

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CatKind {
    Image,
    Video,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatPack {
    /// File or folder name.
    id: String,
    name: String,
    /// The file to show.
    path: String,
    kind: CatKind,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSet {
    /// Folder name.
    id: String,
    name: String,
    /// Icon key (file name without extension) to the file.
    files: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFileError {
    file: String,
    error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFiles {
    themes: Vec<CustomTheme>,
    errors: Vec<ThemeFileError>,
}

/// Creates the folder (so the user can find it) and lets the webview read it.
fn prepare(app: &AppHandle, dir: &Path) -> CommandResult<()> {
    std::fs::create_dir_all(dir).at(dir)?;
    app.asset_protocol_scope().allow_directory(dir, true)?;
    Ok(())
}

/// Directory entries sorted by file name.
fn sorted_entries(dir: &Path) -> k0t1k_core::Result<Vec<std::fs::DirEntry>> {
    let mut entries = std::fs::read_dir(dir).at(dir)?.collect::<Result<Vec<_>, _>>().at(dir)?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    Ok(entries)
}

fn extension_in(path: &Path, allowed: &[&str]) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| allowed.iter().any(|ok| ok.eq_ignore_ascii_case(extension)))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

fn kind_of(path: &Path) -> Option<CatKind> {
    if extension_in(path, &IMAGE_EXTENSIONS) {
        Some(CatKind::Image)
    } else if extension_in(path, &VIDEO_EXTENSIONS) {
        Some(CatKind::Video)
    } else {
        None
    }
}

#[tauri::command]
pub async fn list_icon_library(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    let dir = state.paths.icons_dir();
    prepare(&app, &dir)?;
    blocking(move || {
        Ok(sorted_entries(&dir)?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && extension_in(path, &IMAGE_EXTENSIONS))
            .map(|path| path.display().to_string())
            .collect())
    })
    .await
}

/// Prism's cat pack index: `{ name, default, variants: [{ path, startTime, endTime }] }`; the paths are
/// relative to the folder and a variant is used on the days between its start and end (every year).
#[derive(Deserialize)]
struct CatPackIndex {
    default: Option<String>,
    #[serde(default)]
    variants: Vec<CatVariant>,
}

#[derive(Deserialize)]
struct CatVariant {
    path: String,
    #[serde(rename = "startTime")]
    start: DayOfYear,
    #[serde(rename = "endTime")]
    end: DayOfYear,
}

#[derive(Deserialize, PartialEq, PartialOrd, Clone, Copy)]
struct DayOfYear {
    month: u32,
    day: u32,
}

impl CatVariant {
    /// A range whose start is after its end wraps around the new year.
    fn covers(&self, today: DayOfYear) -> bool {
        if self.start <= self.end {
            self.start <= today && today <= self.end
        } else {
            today >= self.start || today <= self.end
        }
    }
}

/// A path from an index file must stay inside the pack folder.
fn inside(dir: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    let plain = relative.components().all(|part| matches!(part, Component::Normal(_)));
    let path = dir.join(relative);
    (plain && path.is_file() && kind_of(&path).is_some()).then_some(path)
}

fn pick_from_index(dir: &Path, today: NaiveDate) -> Option<PathBuf> {
    let index: CatPackIndex = serde_json::from_slice(&std::fs::read(dir.join(CAT_PACK_INDEX)).ok()?)
        .inspect_err(|error| tracing::warn!("invalid {CAT_PACK_INDEX} in {}: {error}", dir.display()))
        .ok()?;
    let today = DayOfYear { month: today.month(), day: today.day() };
    index
        .variants
        .iter()
        .filter(|variant| variant.covers(today))
        .find_map(|variant| inside(dir, &variant.path))
        .or_else(|| index.default.as_deref().and_then(|default| inside(dir, default)))
}

fn first_media(dir: &Path) -> Option<PathBuf> {
    sorted_entries(dir).ok()?.into_iter().map(|entry| entry.path()).find(|path| path.is_file() && kind_of(path).is_some())
}

fn cat_pack_of(path: &Path, today: NaiveDate) -> Option<CatPack> {
    let (file, name) = if path.is_dir() {
        (pick_from_index(path, today).or_else(|| first_media(path))?, file_name(path))
    } else {
        (path.to_path_buf(), file_stem(path))
    };
    Some(CatPack { id: file_name(path), name, kind: kind_of(&file)?, path: file.display().to_string() })
}

#[tauri::command]
pub async fn list_cat_packs(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Vec<CatPack>> {
    let dir = state.paths.catpacks_dir();
    prepare(&app, &dir)?;
    let today = Local::now().date_naive();
    blocking(move || Ok(sorted_entries(&dir)?.into_iter().filter_map(|entry| cat_pack_of(&entry.path(), today)).collect())).await
}

fn icon_set_of(dir: &Path) -> Option<IconSet> {
    let mut files = BTreeMap::new();
    for entry in sorted_entries(dir).ok()? {
        let path = entry.path();
        if path.is_file() && extension_in(&path, &ICON_SET_EXTENSIONS) {
            files.entry(file_stem(&path)).or_insert_with(|| path.display().to_string());
        }
    }
    (!files.is_empty()).then(|| IconSet { id: file_name(dir), name: file_name(dir), files })
}

#[tauri::command]
pub async fn list_icon_sets(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Vec<IconSet>> {
    let dir = state.paths.iconsets_dir();
    prepare(&app, &dir)?;
    blocking(move || Ok(sorted_entries(&dir)?.into_iter().map(|entry| entry.path()).filter(|path| path.is_dir()).filter_map(|path| icon_set_of(&path)).collect())).await
}

/// A theme file holds a custom theme without its id, which is made from the file name.
fn read_theme(path: &Path) -> Result<CustomTheme, String> {
    let stem = file_stem(path);
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let object = value.as_object_mut().ok_or("the file must contain a JSON object")?;
    object.insert("id".to_owned(), format!("file:{stem}").into());
    object.entry("name").or_insert_with(|| stem.into());
    let theme: CustomTheme = serde_json::from_value(value).map_err(|error| error.to_string())?;
    UiSettings { custom_themes: vec![theme.clone()], ..UiSettings::default() }.validate().map_err(|error| error.to_string())?;
    Ok(theme)
}

#[tauri::command]
pub async fn list_theme_files(state: State<'_, AppState>) -> CommandResult<ThemeFiles> {
    let dir = state.paths.themes_dir();
    blocking(move || {
        std::fs::create_dir_all(&dir).at(&dir)?;
        let mut files = ThemeFiles { themes: Vec::new(), errors: Vec::new() };
        for entry in sorted_entries(&dir)? {
            let path = entry.path();
            if !path.is_file() || !extension_in(&path, &["json"]) {
                continue;
            }
            match read_theme(&path) {
                Ok(theme) => files.themes.push(theme),
                Err(error) => files.errors.push(ThemeFileError { file: file_name(&path), error }),
            }
        }
        Ok(files)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn cat_pack_folder_picks_the_variant_of_today_or_the_default() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["cat.png", "xmas.gif", "ny.webm"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        std::fs::write(
            dir.path().join(CAT_PACK_INDEX),
            r#"{"name":"P","default":"cat.png","variants":[
                {"path":"xmas.gif","startTime":{"day":24,"month":12},"endTime":{"day":26,"month":12}},
                {"path":"ny.webm","startTime":{"day":30,"month":12},"endTime":{"day":2,"month":1}},
                {"path":"../escape.png","startTime":{"day":1,"month":6},"endTime":{"day":30,"month":6}}]}"#,
        )
        .unwrap();
        let file = |today| file_name(Path::new(&cat_pack_of(dir.path(), today).unwrap().path));
        assert_eq!(file(date(12, 25)), "xmas.gif");
        assert_eq!(file(date(1, 1)), "ny.webm");
        assert_eq!(file(date(7, 1)), "cat.png");
        assert_eq!(file(date(6, 10)), "cat.png");
    }

    #[test]
    fn cat_pack_without_index_uses_the_first_media_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("b.mp4"), b"x").unwrap();
        let pack = cat_pack_of(dir.path(), date(1, 1)).unwrap();
        assert!(matches!(pack.kind, CatKind::Video));
        assert!(pack.path.ends_with("b.mp4"));
        let empty = tempfile::tempdir().unwrap();
        assert!(cat_pack_of(empty.path(), date(1, 1)).is_none());
    }

    #[test]
    fn theme_file_id_comes_from_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("night.json");
        std::fs::write(&good, r##"{"base":"k0t1k","colors":{"accent":"#fff"}}"##).unwrap();
        let theme = read_theme(&good).unwrap();
        assert_eq!((theme.id.as_str(), theme.name.as_str()), ("file:night", "night"));
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, r#"{"base":"k0t1k","colors":{"accent":"red;x"}}"#).unwrap();
        assert!(read_theme(&bad).is_err());
        std::fs::write(&bad, "[]").unwrap();
        assert!(read_theme(&bad).is_err());
    }

    #[test]
    fn icon_set_maps_stems_to_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("play.svg"), b"x").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        let set = icon_set_of(dir.path()).unwrap();
        assert_eq!(set.files.keys().collect::<Vec<_>>(), ["play"]);
    }
}
