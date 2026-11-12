//! Full-bundle macOS updates. Nothing in the installed bundle is modified in place.
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use k0t1k_core::i18n::tr;
use k0t1k_core::tr_fmt;
use k0t1k_core::update::{self, ReleaseInfo, macos as bundle};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::error::{AppError, CommandResult};
use crate::events::{progress_fn, tracked};
use crate::onboarding::{WAIT_FOR, spawn_detached};

pub(crate) const FINISH_UPDATE: &str = "--finish-macos-update";
const STAGE_PREFIX: &str = ".k0t1k-update-";
const MARKER: &str = "update-owner.json";

#[derive(Serialize, Deserialize)]
struct Owner {
    bundle: PathBuf,
    previous_pid: u32,
}

pub(super) struct Prepared {
    stage: tempfile::TempDir,
    installed: PathBuf,
    extracted: PathBuf,
}

pub(super) fn launcher_bundle() -> CommandResult<PathBuf> {
    let executable = std::env::current_exe()?;
    let app = bundle::bundle_from_executable(&executable).map_err(|_| {
        AppError::new(tr(
            "Updates require an installed .app. Drag the launcher from the DMG into Applications first.",
            "Для обновлений нужен установленный .app. Сначала перенесите лаунчер из DMG в «Программы».",
        ))
    })?;
    let app = fs::canonicalize(app)?;
    // This legacy override lives inside the signed bundle, not in the profile. Replacing it
    // would lose the setting; copying it into the new bundle would change the verified package.
    if crate::datadir::override_file(&app.join("Contents/MacOS")).try_exists()? {
        return Err(AppError::new(tr(
            "This .app contains a custom data-dir.txt. Update it manually while preserving that setting; automatic replacement is blocked to protect your data.",
            "Внутри .app есть пользовательский data-dir.txt. Обновите приложение вручную с сохранением этой настройки; автоматическая замена заблокирована для защиты данных.",
        )));
    }
    if app.components().any(|part| part.as_os_str() == "AppTranslocation") {
        return Err(AppError::new(tr(
            "Move the launcher into Applications and open it there before updating.",
            "Перед обновлением перенесите лаунчер в «Программы» и откройте его оттуда.",
        )));
    }
    Ok(app)
}

fn new_stage(installed: &Path) -> CommandResult<tempfile::TempDir> {
    let parent = installed.parent().ok_or_else(|| AppError::new("the application has no parent folder"))?;
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .prefix(STAGE_PREFIX)
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)
        .map_err(|error| {
            AppError::new(tr_fmt!(
                "Cannot write beside the application: {error}. Move it from the DMG to a writable Applications folder.",
                "Нельзя писать рядом с приложением: {error}. Перенесите его из DMG в доступную для записи папку «Программы».",
            ))
        })
}

pub(super) fn install_blocked() -> Option<String> {
    launcher_bundle().and_then(|app| new_stage(&app).map(drop)).err().map(|error| error.to_string())
}

fn plist_value(app: &Path, key: &str) -> CommandResult<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(app.join("Contents/Info.plist"))
        .output()?;
    if !output.status.success() {
        return Err(AppError::new(format!("cannot read application {key}: {}", String::from_utf8_lossy(&output.stderr))));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn validate(app: &Path, version: &str) -> CommandResult<()> {
    bundle::validate_bundle_binary(app, std::env::consts::ARCH)?;
    for (key, expected) in [
        ("CFBundleIdentifier", "dev.k0t1k.launcher"),
        ("CFBundleShortVersionString", version),
        ("CFBundleExecutable", "k0t1k-app"),
    ] {
        if plist_value(app, key)? != expected {
            return Err(AppError::new(format!("the update has an unexpected {key}")));
        }
    }
    let output = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(app).output()?;
    if !output.status.success() {
        return Err(AppError::new(format!(
            "application code signature verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

pub(super) async fn prepare(app: &AppHandle, client: &reqwest::Client, release: &ReleaseInfo, current: &str) -> CommandResult<Prepared> {
    let installed = launcher_bundle()?;
    let stage = new_stage(&installed)?;
    let archive = stage.path().join("download.zip");
    let title = tr("Launcher update", "Обновление лаунчера");
    let progress = progress_fn(app, update::TASK, title);
    tracked(app, update::TASK, title, async {
        update::download(client, release, current, &archive, &progress)
            .await
            .map_err(|error| super::failure(&error))
    })
    .await?;
    let version = release.version.clone();
    tokio::task::spawn_blocking(move || {
        let destination = stage.path().join("extracted");
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&destination)?;
        let extracted = bundle::extract_bundle(&archive, &destination)?;
        validate(&extracted, &version)?;
        let marker = Owner {
            bundle: installed.clone(),
            previous_pid: std::process::id(),
        };
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(stage.path().join(MARKER))?;
        serde_json::to_writer(&mut file, &marker)?;
        file.sync_all()?;
        Ok::<_, AppError>(Prepared { stage, installed, extracted })
    })
    .await?
    .map_err(|error| {
        tracing::warn!("macOS update preparation failed: {error}");
        AppError::new(tr(
            "The macOS application update could not be verified. The installed launcher was not changed. Details are in launcher.log.",
            "Не удалось проверить обновление приложения macOS. Установленный лаунчер не изменён. Подробности в launcher.log.",
        ))
    })
}

fn recovery_error(installed: &Path, old: &Path, cause: &dyn std::fmt::Display) -> AppError {
    AppError::new(tr_fmt!(
        "The update failed ({cause}). The recovery application is at {old}. If {installed} already exists, move it elsewhere first, then move {old} to {installed}. Do not delete the recovery copy.",
        "Обновление не удалось ({cause}). Резервное приложение находится в {old}. Если {installed} уже существует, сначала переместите его в другое место, затем перенесите {old} в {installed}. Не удаляйте резервную копию.",
        old = old.display(),
        installed = installed.display(),
    ))
}

fn cleanup_failed_stage(stage: &Path) {
    if let Err(error) = fs::remove_dir_all(stage) {
        tracing::warn!("cannot remove failed update staging {}: {error}", stage.display());
    }
}

/// No automatic TempDir cleanup is allowed once the backup may be the only working application.
fn replace_bundle(installed: &Path, extracted: &Path, stage: &Path, start: impl FnOnce(&Path) -> CommandResult<()>) -> CommandResult<()> {
    let old = stage.join("previous.app");
    if let Err(error) = fs::rename(installed, &old) {
        cleanup_failed_stage(stage);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(extracted, installed) {
        return match fs::rename(&old, installed) {
            Ok(()) => {
                cleanup_failed_stage(stage);
                Err(error.into())
            }
            Err(rollback) => Err(recovery_error(installed, &old, &format!("{error}; {rollback}"))),
        };
    }
    if let Err(error) = start(&installed.join(bundle::EXECUTABLE_RELATIVE)) {
        // Directories cannot be renamed over a nonempty directory. Keep the rejected update away
        // before restoring the original, and keep every copy if either rollback operation fails.
        if let Err(rollback) = fs::rename(installed, extracted).and_then(|()| fs::rename(&old, installed)) {
            return Err(recovery_error(installed, &old, &format!("{error}; {rollback}")));
        }
        cleanup_failed_stage(stage);
        return Err(error);
    }
    Ok(())
}

pub(super) fn replace_and_restart(prepared: Prepared) -> CommandResult<()> {
    let Prepared { stage, installed, extracted } = prepared;
    let stage = stage.keep();
    let args = [
        OsString::from(WAIT_FOR),
        std::process::id().to_string().into(),
        OsString::from(FINISH_UPDATE),
        stage.as_os_str().to_owned(),
    ];
    replace_bundle(&installed, &extracted, &stage, |exe| spawn_detached(exe, &args))
}

fn cleanup(stage: &Path, installed: &Path, previous_pid: u32) -> CommandResult<()> {
    let installed = fs::canonicalize(installed)?;
    let metadata = fs::symlink_metadata(stage)?;
    let is_stage = stage.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(STAGE_PREFIX));
    let canonical = fs::canonicalize(stage)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !is_stage
        || canonical != stage
        || canonical.parent() != installed.parent()
        || metadata.uid() != fs::metadata(&installed)?.uid()
    {
        return Err(AppError::new("refusing to clean an unowned macOS update directory"));
    }
    let marker = stage.join(MARKER);
    let metadata = fs::symlink_metadata(&marker)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err(AppError::new("invalid macOS update ownership marker"));
    }
    let owner: Owner = serde_json::from_slice(&fs::read(marker)?)?;
    if owner.bundle != installed || owner.previous_pid != previous_pid || previous_pid == 0 {
        return Err(AppError::new("macOS update directory belongs to another application or process"));
    }
    fs::remove_dir_all(stage)?;
    Ok(())
}

pub(crate) fn finish_update(stage: &Path, previous_pid: u32) -> CommandResult<()> {
    cleanup(stage, &launcher_bundle()?, previous_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let root_path = fs::canonicalize(root.path()).unwrap();
        let installed = root_path.join("Renamed launcher.app");
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("version"), "old").unwrap();
        let stage = new_stage(&installed).unwrap().keep();
        let extracted = stage.join("next.app");
        fs::create_dir(&extracted).unwrap();
        fs::write(extracted.join("version"), "new").unwrap();
        let owner = Owner {
            bundle: installed.clone(),
            previous_pid: 42,
        };
        fs::write(stage.join(MARKER), serde_json::to_vec(&owner).unwrap()).unwrap();
        (root, installed, stage, extracted)
    }

    #[test]
    fn replaces_whole_bundle_and_only_cleans_owned_backup() {
        let (_root, installed, stage, extracted) = fixture();
        replace_bundle(&installed, &extracted, &stage, |_| Ok(())).unwrap();
        assert_eq!(fs::read_to_string(installed.join("version")).unwrap(), "new");
        assert_eq!(fs::read_to_string(stage.join("previous.app/version")).unwrap(), "old");
        assert!(cleanup(&stage, &installed, 43).is_err());
        assert!(stage.exists());
        cleanup(&stage, &installed, 42).unwrap();
        assert!(!stage.exists());
        assert!(installed.exists());
    }

    #[test]
    fn extraction_or_spawn_failure_restores_original_bundle() {
        for missing in [false, true] {
            let (_root, installed, stage, extracted) = fixture();
            if missing {
                fs::remove_dir_all(&extracted).unwrap();
            }
            assert!(replace_bundle(&installed, &extracted, &stage, |_| Err(AppError::new("cannot start"))).is_err());
            assert_eq!(fs::read_to_string(installed.join("version")).unwrap(), "old");
            assert!(!stage.exists());
        }
    }

    #[test]
    fn failed_rollback_preserves_recovery_bundle() {
        let (_root, installed, stage, extracted) = fixture();
        let error = replace_bundle(&installed, &extracted, &stage, |_| {
            fs::create_dir(&extracted).unwrap();
            fs::write(extracted.join("collision"), "keep").unwrap();
            Err(AppError::new("cannot start"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("previous.app"));
        assert_eq!(fs::read_to_string(stage.join("previous.app/version")).unwrap(), "old");
        assert_eq!(fs::read_to_string(installed.join("version")).unwrap(), "new");
    }

    #[test]
    #[ignore = "requires K0T1K_TEST_UPDATE_ZIP pointing to a native release bundle"]
    fn validates_packaged_release_bundle() {
        use std::os::unix::fs::PermissionsExt;
        let archive = PathBuf::from(std::env::var_os("K0T1K_TEST_UPDATE_ZIP").expect("set the release ZIP path"));
        let destination = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let app = bundle::extract_bundle(&archive, destination.path()).unwrap();
        validate(&app, env!("CARGO_PKG_VERSION")).unwrap();
    }

    #[test]
    fn cleanup_rejects_unmarked_wrong_parent_and_symlink_paths() {
        let (_root, installed, stage, _extracted) = fixture();
        let other = tempfile::tempdir().unwrap();
        assert!(cleanup(&stage, other.path(), 42).is_err());
        let link = stage.with_extension("link");
        std::os::unix::fs::symlink(&stage, &link).unwrap();
        assert!(cleanup(&link, &installed, 42).is_err());
        fs::remove_file(stage.join(MARKER)).unwrap();
        assert!(cleanup(&stage, &installed, 42).is_err());
        assert!(stage.exists() && installed.exists());
    }
}
