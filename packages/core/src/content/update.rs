//! Updating an instance in place to another version of the modpack it was installed from.

use std::path::Path;

use super::modpack::{InstallTarget, apply_modpack};
use super::pack_state::{Backup, PackState};
use super::{Clients, ModpackFormat, ModpackInstallReport, ModpackManifest};
use crate::error::{Error, Result};
use crate::progress::ProgressFn;

/// Applies `manifest` (the new pack version, read from `zip_path`) to the instance. Files the old
/// pack brought and the new one dropped are removed (also their metadata), changed ones are replaced,
/// unchanged ones are not downloaded again, and anything the user added is never touched. Overrides
/// the old pack unpacked and the new one no longer has are deleted; the new ones are unpacked over.
///
/// The `mods` folder and every other path the update changes are zipped into `<instance>/backups/`
/// first. If the update fails halfway, they are restored and the original error is returned. On
/// success the new pack state is saved; changing the instance's components is the caller's job.
pub async fn update_modpack(
    manifest: &ModpackManifest,
    zip_path: &Path,
    instance_dir: &Path,
    game_dir: &Path,
    clients: &Clients,
    progress: &ProgressFn,
) -> Result<ModpackInstallReport> {
    if manifest.format == ModpackFormat::Instance {
        return Err(Error::Other("only Modrinth and CurseForge packs can be updated".into()));
    }
    let previous = PackState::load(instance_dir)?;
    let mut backup: Option<Backup> = None;
    let mut target = InstallTarget {
        game_dir,
        previous: &previous,
        before_apply: &mut |touched| {
            backup = Some(Backup::create(instance_dir, game_dir, &["mods"], touched)?);
            Ok(())
        },
    };
    let applied = match apply_modpack(manifest, zip_path, &mut target, clients, progress).await {
        Ok(report) => report.state.save(instance_dir).map(|()| report),
        Err(error) => Err(error),
    };
    match (applied, backup) {
        (Ok(report), _) => Ok(report),
        (Err(error), None) => Err(error),
        (Err(error), Some(backup)) => match backup.restore() {
            Ok(()) => Err(error),
            Err(restore) => Err(Error::Other(format!(
                "{error}; restoring the backup also failed, it is still in the instance's backups folder: {restore}"
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;
    use crate::content::inspect_modpack;
    use crate::content::pack_state::PackFileState;
    use crate::progress::no_progress;

    fn mrpack(dir: &Path, name: &str, files_json: &str, overrides: &[(&str, &str)]) -> PathBuf {
        let index = format!(
            r#"{{"formatVersion":1,"game":"minecraft","versionId":"2","name":"n","files":{files_json},"dependencies":{{"minecraft":"1.21.4"}}}}"#
        );
        let path = dir.join(name);
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("modrinth.index.json", options).unwrap();
        writer.write_all(index.as_bytes()).unwrap();
        for (entry, content) in overrides {
            writer.start_file(format!("overrides/{entry}"), options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn write(game_dir: &Path, relative: &str, content: &str) {
        let path = game_dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// An instance the old pack installed `mods/old.jar` (+ metadata) and `config/old.toml` into,
    /// next to a mod and a config the user added.
    fn old_instance(dir: &Path) -> (PathBuf, PathBuf) {
        let (instance_dir, game_dir) = (dir.join("instance"), dir.join("instance/minecraft"));
        write(&game_dir, "mods/old.jar", "old");
        write(&game_dir, "mods/old.jar.bak", "x");
        write(&game_dir, "mods/user.jar", "mine");
        write(&game_dir, "mods/.index/old.pw.toml", "name = \"Old\"\nfilename = \"old.jar\"\n");
        write(&game_dir, "config/old.toml", "old");
        write(&game_dir, "config/kept.toml", "old");
        write(&game_dir, "config/user.toml", "mine");
        PackState {
            files: vec![PackFileState { path: "mods/old.jar".into(), id: "aa".into() }],
            overrides: vec!["config/old.toml".into(), "config/kept.toml".into()],
        }
        .save(&instance_dir)
        .unwrap();
        (instance_dir, game_dir)
    }

    #[tokio::test]
    async fn update_removes_dropped_pack_files_and_keeps_what_the_user_added() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (instance_dir, game_dir) = old_instance(dir.path());
        let zip = mrpack(dir.path(), "new.mrpack", "[]", &[("config/kept.toml", "new"), ("config/added.toml", "new")]);
        let manifest = inspect_modpack(&zip).unwrap();

        update_modpack(&manifest, &zip, &instance_dir, &game_dir, &clients, &no_progress()).await.unwrap();

        assert!(!game_dir.join("mods/old.jar").exists(), "a mod the new pack dropped is removed");
        assert!(!game_dir.join("mods/.index/old.pw.toml").exists(), "with its metadata");
        assert!(!game_dir.join("config/old.toml").exists(), "a removed override is cleaned");
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/user.jar")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/old.jar.bak")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(game_dir.join("config/user.toml")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(game_dir.join("config/kept.toml")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(game_dir.join("config/added.toml")).unwrap(), "new");

        let state = PackState::load(&instance_dir).unwrap();
        assert!(state.files.is_empty());
        assert_eq!(state.overrides, ["config/added.toml", "config/kept.toml"]);
        assert_eq!(std::fs::read_dir(instance_dir.join("backups")).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_failed_update_is_rolled_back() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (instance_dir, game_dir) = old_instance(dir.path());
        // The new pack needs a download that cannot succeed, after the old files are already removed.
        let files = r#"[{"path":"mods/new.jar","hashes":{"sha1":"a","sha512":"b"},"downloads":["http://127.0.0.1:1/new.jar"],"fileSize":3}]"#;
        let zip = mrpack(dir.path(), "new.mrpack", files, &[("config/added.toml", "new")]);
        let manifest = inspect_modpack(&zip).unwrap();

        let error = update_modpack(&manifest, &zip, &instance_dir, &game_dir, &clients, &no_progress()).await;
        assert!(error.is_err());

        assert_eq!(std::fs::read_to_string(game_dir.join("mods/old.jar")).unwrap(), "old");
        assert!(game_dir.join("mods/.index/old.pw.toml").exists());
        assert_eq!(std::fs::read_to_string(game_dir.join("mods/user.jar")).unwrap(), "mine");
        assert!(!game_dir.join("mods/new.jar").exists());
        assert_eq!(std::fs::read_to_string(game_dir.join("config/old.toml")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(game_dir.join("config/kept.toml")).unwrap(), "old");
        assert!(!game_dir.join("config/added.toml").exists());
        let state = PackState::load(&instance_dir).unwrap();
        assert_eq!(state.files.len(), 1, "the old state is kept");
        assert_eq!(state.files[0].path, "mods/old.jar");
    }

    #[tokio::test]
    async fn an_unchanged_file_is_not_downloaded_again_and_stays_disabled() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (instance_dir, game_dir) = old_instance(dir.path());
        std::fs::rename(game_dir.join("mods/old.jar"), game_dir.join("mods/old.jar.disabled")).unwrap();
        // Same path and sha512 as the installed state; the URL would fail if it were fetched.
        let files = r#"[{"path":"mods/old.jar","hashes":{"sha1":"a","sha512":"aa"},"downloads":["http://127.0.0.1:1/old.jar"],"fileSize":3}]"#;
        let zip = mrpack(dir.path(), "new.mrpack", files, &[]);
        let manifest = inspect_modpack(&zip).unwrap();

        update_modpack(&manifest, &zip, &instance_dir, &game_dir, &clients, &no_progress()).await.unwrap();
        assert!(game_dir.join("mods/old.jar.disabled").exists());
        assert!(!game_dir.join("mods/old.jar").exists());
        assert_eq!(PackState::load(&instance_dir).unwrap().files.len(), 1);
    }

    #[tokio::test]
    async fn instance_zips_cannot_be_updated() {
        let clients = Clients::new(reqwest::Client::new(), None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("i.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("instance.json", options).unwrap();
        writer
            .write_all(br#"{"name":"A","components":[{"uid":"net.minecraft","version":"1.21.1"}],"created":"2026-01-01T00:00:00Z"}"#)
            .unwrap();
        writer.finish().unwrap();
        let manifest = inspect_modpack(&path).unwrap();
        assert!(update_modpack(&manifest, &path, dir.path(), dir.path(), &clients, &no_progress()).await.is_err());
    }
}
