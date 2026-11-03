//! Moving one account to another PC: its tokens go into a file encrypted with a password (age, scrypt), because
//! `accounts.json` itself is bound to this Windows user by DPAPI.

use std::path::Path;

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::{Secrets, SecretsFile, save_account};
use crate::error::{Error, IoContext, Result};
use crate::i18n::tr;
use crate::paths::Paths;
use crate::settings::{Account, GlobalSettings};
use crate::tr_fmt;

/// Shortest password accepted for an export file; the file holds refresh tokens, so it must resist guessing.
pub const MIN_PASSWORD_LEN: usize = 10;

/// An export file is a few hundred bytes; anything larger is not one and is not read into memory.
const MAX_FILE_SIZE: u64 = 256 * 1024;

/// Version of the payload, so a future format can be told apart from this one.
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    secrets: Secrets,
}

/// What [`import_account`] did.
#[derive(Debug)]
pub enum ImportOutcome {
    Imported(Account),
    /// An account with this id is already signed in and `overwrite` was not set; nothing was changed.
    AlreadyExists(Account),
}

/// Writes the tokens of the account `account_id` (and nothing else) to `dest`, encrypted with `password`.
pub fn export_account(paths: &Paths, account_id: &str, password: &str, dest: &Path) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::Other(tr_fmt!(
            "The password must be at least {} characters",
            "Пароль должен быть не короче {} символов",
            MIN_PASSWORD_LEN
        )));
    }
    let secrets = SecretsFile::load(paths)?
        .accounts
        .remove(account_id)
        .ok_or_else(|| Error::Other(tr("This account has no stored sign-in to export", "У этого аккаунта нет сохранённого входа для экспорта").to_owned()))?;
    let plain = serde_json::to_vec(&Payload { version: FORMAT_VERSION, secrets })?;
    let encrypted = age::encrypt(&age::scrypt::Recipient::new(SecretString::from(password.to_owned())), &plain)
        .map_err(|error| Error::Other(format!("cannot encrypt the account: {error}")))?;
    std::fs::write(dest, encrypted).at(dest)
}

/// Reads an export file and stores its account like a fresh sign-in. An account that is already signed in is only
/// replaced when `overwrite` is set.
pub fn import_account(paths: &Paths, password: &str, src: &Path, overwrite: bool) -> Result<ImportOutcome> {
    let size = std::fs::metadata(src).at(src)?.len();
    if size > MAX_FILE_SIZE {
        return Err(Error::Other(tr("This is not an account file (too large)", "Это не файл аккаунта (слишком большой)").to_owned()));
    }
    let encrypted = std::fs::read(src).at(src)?;
    // One message for a wrong password and a damaged file: age cannot tell them apart, and neither should the UI.
    let damaged = || Error::Other(tr("Wrong password or the file is damaged", "Неверный пароль или файл повреждён").to_owned());
    let plain = age::decrypt(&age::scrypt::Identity::new(SecretString::from(password.to_owned())), &encrypted).map_err(|_| damaged())?;
    let payload: Payload = serde_json::from_slice(&plain).map_err(|_| damaged())?;
    if payload.version != FORMAT_VERSION {
        return Err(Error::Other(tr(
            "This account file was made by a newer launcher version",
            "Этот файл аккаунта создан более новой версией лаунчера",
        )
        .to_owned()));
    }
    let account = payload.secrets.public();
    if !overwrite && GlobalSettings::load(paths)?.accounts.iter().any(|existing| existing.id == account.id) {
        return Ok(ImportOutcome::AlreadyExists(account));
    }
    save_account(paths, payload.secrets).map(ImportOutcome::Imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MicrosoftSecrets;

    fn secrets(uuid: &str, refresh: &str) -> Secrets {
        Secrets::Microsoft(MicrosoftSecrets {
            uuid: uuid.to_owned(),
            name: "Steve".to_owned(),
            refresh_token: refresh.to_owned(),
            access_token: "access-secret".to_owned(),
            expires_at: 0,
        })
    }

    #[test]
    fn exported_account_imports_on_another_pc_and_only_that_account_travels() {
        let (source, target) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (from, to) = (Paths::new(source.path()), Paths::new(target.path()));
        save_account(&from, secrets("abc", "refresh-secret")).unwrap();
        save_account(&from, secrets("other", "other-secret")).unwrap();
        let file = source.path().join("a.k0t1k-auth");

        export_account(&from, "ms-abc", "correct horse battery", &file).unwrap();
        let raw = std::fs::read(&file).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("refresh-secret"));

        let ImportOutcome::Imported(account) = import_account(&to, "correct horse battery", &file, false).unwrap() else {
            panic!("expected an import");
        };
        assert_eq!(account.id, "ms-abc");
        assert_eq!(SecretsFile::load(&to).unwrap().accounts.keys().collect::<Vec<_>>(), ["ms-abc"]);
        assert_eq!(GlobalSettings::load(&to).unwrap().accounts.len(), 1);
    }

    #[test]
    fn existing_account_is_replaced_only_with_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, secrets("abc", "old-refresh")).unwrap();
        let file = dir.path().join("a.k0t1k-auth");
        export_account(&paths, "ms-abc", "correct horse battery", &file).unwrap();
        save_account(&paths, secrets("abc", "new-refresh")).unwrap();

        let outcome = import_account(&paths, "correct horse battery", &file, false).unwrap();
        assert!(matches!(outcome, ImportOutcome::AlreadyExists(_)));
        let refresh = |paths: &Paths| match SecretsFile::load(paths).unwrap().accounts.remove("ms-abc") {
            Some(Secrets::Microsoft(secrets)) => secrets.refresh_token,
            _ => panic!("missing account"),
        };
        assert_eq!(refresh(&paths), "new-refresh");

        assert!(matches!(import_account(&paths, "correct horse battery", &file, true).unwrap(), ImportOutcome::Imported(_)));
        assert_eq!(refresh(&paths), "old-refresh");
    }

    #[test]
    fn rejects_short_password_unknown_account_wrong_password_and_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, secrets("abc", "r")).unwrap();
        let file = dir.path().join("a.k0t1k-auth");

        assert!(export_account(&paths, "ms-abc", "short", &file).is_err());
        assert!(export_account(&paths, "ms-nope", "correct horse battery", &file).is_err());
        assert!(!file.exists());

        export_account(&paths, "ms-abc", "correct horse battery", &file).unwrap();
        let error = import_account(&paths, "wrong password!!", &file, false).unwrap_err();
        assert!(error.to_string().contains("Wrong password"));

        let big = dir.path().join("big");
        std::fs::write(&big, vec![0u8; MAX_FILE_SIZE as usize + 1]).unwrap();
        assert!(import_account(&paths, "correct horse battery", &big, false).is_err());
    }
}
