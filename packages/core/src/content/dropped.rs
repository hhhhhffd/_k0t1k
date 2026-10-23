//! Files the user drops onto an instance: identify what they are (the way Prism's
//! `LocalResourceParse` does) and copy them into the right game folder.

use std::fs::File;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::ResourceKind;
use crate::error::{IoContext, Result};
use crate::instance::fsutil::unique_path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedFile {
    pub file_name: String,
    pub kind: ResourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    NotAFile,
    /// The folder is fixed (a tab) and the file's extension does not fit it.
    UnsupportedType,
    /// Nothing in the file says what it is.
    UnknownType,
    /// Identified as a world or a data pack, which are not added by dropping.
    WorldOrDataPack,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFilesReport {
    pub added: Vec<AddedFile>,
    pub skipped: Vec<SkippedFile>,
}

fn extension_of(path: &Path) -> String {
    path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// What a file is: a `.jar` or `.litemod` is a mod; a zip is a shader pack (has `shaders/`), a world
/// (has `level.dat`), a data pack (`pack.mcmeta` with `data/` and no `assets/`) or a resource pack
/// (`pack.mcmeta`). `None` when it is none of these.
pub fn identify_file(path: &Path) -> Result<Option<ResourceKind>> {
    match extension_of(path).as_str() {
        "jar" | "litemod" => Ok(Some(ResourceKind::Mod)),
        "zip" => identify_zip(path),
        _ => Ok(None),
    }
}

fn identify_zip(path: &Path) -> Result<Option<ResourceKind>> {
    let mut archive = zip::ZipArchive::new(File::open(path).at(path)?)?;
    let (mut shaders, mut level, mut mcmeta, mut assets, mut data) = (false, false, false, false, false);
    for index in 0..archive.len() {
        let name = archive.by_index(index)?.name()?.into_owned();
        shaders |= name.starts_with("shaders/");
        level |= name == "level.dat";
        mcmeta |= name == "pack.mcmeta";
        assets |= name.starts_with("assets/");
        data |= name.starts_with("data/");
    }
    Ok(if shaders {
        Some(ResourceKind::ShaderPack)
    } else if level {
        Some(ResourceKind::World)
    } else if mcmeta && data && !assets {
        Some(ResourceKind::DataPack)
    } else if mcmeta {
        Some(ResourceKind::ResourcePack)
    } else {
        None
    })
}

fn accepts(kind: ResourceKind, path: &Path) -> bool {
    match kind {
        ResourceKind::Mod => ["jar", "litemod", "zip"].contains(&extension_of(path).as_str()),
        ResourceKind::ResourcePack | ResourceKind::ShaderPack => extension_of(path) == "zip",
        _ => false,
    }
}

/// Copies dropped files into the game dir. With `kind`, every file goes into that kind's folder
/// (when its extension fits); with `None` the type is identified per file, and only mods, resource
/// packs and shader packs are taken. Files that cannot be placed are returned in `skipped` with the
/// reason; a name that is already taken gets ` (n)` appended, nothing is overwritten.
pub fn add_local_files(game_dir: &Path, kind: Option<ResourceKind>, paths: &[PathBuf]) -> Result<AddFilesReport> {
    let mut report = AddFilesReport::default();
    for path in paths {
        let skip = |reason| SkippedFile { path: path.display().to_string(), reason };
        let Some(file_name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            report.skipped.push(skip(SkipReason::NotAFile));
            continue;
        };
        if !path.is_file() {
            report.skipped.push(skip(SkipReason::NotAFile));
            continue;
        }
        let target_kind = match kind {
            Some(kind) if accepts(kind, path) => kind,
            Some(_) => {
                report.skipped.push(skip(SkipReason::UnsupportedType));
                continue;
            }
            None => match identify_file(path)? {
                Some(found @ (ResourceKind::Mod | ResourceKind::ResourcePack | ResourceKind::ShaderPack)) => found,
                Some(_) => {
                    report.skipped.push(skip(SkipReason::WorldOrDataPack));
                    continue;
                }
                None => {
                    report.skipped.push(skip(SkipReason::UnknownType));
                    continue;
                }
            },
        };
        let folder = game_dir.join(target_kind.folder());
        std::fs::create_dir_all(&folder).at(&folder)?;
        let destination = unique_path(&folder, &file_name);
        std::fs::copy(path, &destination).at(&destination)?;
        report.added.push(AddedFile {
            file_name: destination.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or(file_name),
            kind: target_kind,
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn zip_with(dir: &Path, name: &str, entries: &[&str]) -> PathBuf {
        let path = dir.join(name);
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for entry in entries {
            writer.start_file(*entry, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn identifies_mods_packs_shaders_worlds_and_data_packs() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let kind = |entries: &[&str]| identify_file(&zip_with(d, "a.zip", entries)).unwrap();
        assert_eq!(kind(&["pack.mcmeta", "assets/minecraft/x.png"]), Some(ResourceKind::ResourcePack));
        assert_eq!(kind(&["pack.mcmeta", "data/minecraft/x.json"]), Some(ResourceKind::DataPack));
        assert_eq!(kind(&["pack.mcmeta", "data/a.json", "assets/b.png"]), Some(ResourceKind::ResourcePack));
        assert_eq!(kind(&["shaders/final.fsh"]), Some(ResourceKind::ShaderPack));
        assert_eq!(kind(&["level.dat", "region/r.0.0.mca"]), Some(ResourceKind::World));
        assert_eq!(kind(&["readme.txt"]), None);
        std::fs::write(d.join("m.jar"), "x").unwrap();
        assert_eq!(identify_file(&d.join("m.jar")).unwrap(), Some(ResourceKind::Mod));
        std::fs::write(d.join("n.txt"), "x").unwrap();
        assert_eq!(identify_file(&d.join("n.txt")).unwrap(), None);
    }

    #[test]
    fn dropped_files_are_copied_into_their_folders_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("downloads");
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(game_dir.join("mods")).unwrap();
        std::fs::write(game_dir.join("mods/a.jar"), "mine").unwrap();
        std::fs::write(source.join("a.jar"), "dropped").unwrap();
        let pack = zip_with(&source, "pack.zip", &["pack.mcmeta", "assets/a"]);
        let shader = zip_with(&source, "shader.zip", &["shaders/a.fsh"]);
        let world = zip_with(&source, "world.zip", &["level.dat"]);
        let notes = source.join("notes.txt");
        std::fs::write(&notes, "x").unwrap();

        let report = add_local_files(&game_dir, None, &[source.join("a.jar"), pack, shader, world, notes.clone()]).unwrap();
        let added: Vec<_> = report.added.iter().map(|file| (file.file_name.as_str(), file.kind)).collect();
        assert_eq!(
            added,
            [
                ("a (1).jar", ResourceKind::Mod),
                ("pack.zip", ResourceKind::ResourcePack),
                ("shader.zip", ResourceKind::ShaderPack)
            ]
        );
        let reasons: Vec<_> = report.skipped.iter().map(|file| file.reason).collect();
        assert_eq!(reasons, [SkipReason::WorldOrDataPack, SkipReason::UnknownType]);
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/a.jar")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/a (1).jar")).unwrap(), "dropped");
        assert!(game_dir.join("resourcepacks/pack.zip").is_file() && game_dir.join("shaderpacks/shader.zip").is_file());

        // On a tab the folder is fixed and only the extension is checked.
        let report = add_local_files(&game_dir, Some(ResourceKind::ResourcePack), &[notes, source.join("world.zip")]).unwrap();
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.skipped.len(), 1);
    }
}
