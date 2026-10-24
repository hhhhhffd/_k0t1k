//! Premium accounts: Microsoft (device code flow) and ely.by (Yggdrasil + authlib-injector).
//!
//! The public account list (`{id, name, kind}`) lives in `settings.json`; the tokens live in
//! `accounts.json`, which is never sent to the UI and is encrypted with DPAPI on Windows (see [`protect`]). Account ids: offline accounts use the offline UUID,
//! Microsoft `ms-<uuid>` and ely.by `elyby-<uuid>`, so they cannot collide.

pub mod elyby;
pub mod microsoft;
mod protect;

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::launch::Account as GameAccount;
use crate::paths::Paths;
use crate::settings::{Account, AccountKind, GlobalSettings, write_json_private};

pub use elyby::{ElybySecrets, Login as ElybyLogin};
pub use microsoft::{DeviceCode, MicrosoftSecrets};

/// Serialises read-modify-write cycles of `accounts.json` within this process.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Keeps two launches from refreshing the same Microsoft or ely.by token (which rotates) at once.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Tokens of one account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Secrets {
    Microsoft(MicrosoftSecrets),
    Elyby(ElybySecrets),
}

// Tokens must never end up in logs, so only the identity is printed.
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets").field("id", &self.account_id()).finish_non_exhaustive()
    }
}

impl Secrets {
    pub fn account_id(&self) -> String {
        match self {
            Secrets::Microsoft(secrets) => format!("ms-{}", secrets.uuid),
            Secrets::Elyby(secrets) => format!("elyby-{}", secrets.uuid),
        }
    }

    fn public(&self) -> Account {
        let (name, kind) = match self {
            Secrets::Microsoft(secrets) => (&secrets.name, AccountKind::Microsoft),
            Secrets::Elyby(secrets) => (&secrets.name, AccountKind::Elyby),
        };
        Account { id: self.account_id(), name: name.clone(), kind }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SecretsFile {
    accounts: BTreeMap<String, Secrets>,
}

/// What `accounts.json` holds on disk.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredFile {
    /// Hex of the DPAPI-encrypted JSON of a [`SecretsFile`] (Windows).
    Protected { protected: String },
    /// Plain JSON: other platforms, and files written before the tokens were encrypted.
    Plain(SecretsFile),
}

impl SecretsFile {
    fn load(paths: &Paths) -> Result<Self> {
        Ok(Self::load_stored(paths)?.0)
    }

    /// The tokens and whether the file still holds them unencrypted.
    fn load_stored(paths: &Paths) -> Result<(Self, bool)> {
        let file = paths.accounts_file();
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok((Self::default(), false)),
            Err(source) => return Err(Error::Io { path: file, source }),
        };
        match serde_json::from_slice(&bytes)? {
            StoredFile::Plain(secrets) => Ok((secrets, true)),
            StoredFile::Protected { protected } => Ok((Self::decrypt(&protected), false)),
        }
    }

    /// Tokens encrypted for another Windows user or PC (a moved portable folder) cannot be used anyway: they are
    /// dropped with a warning, launches then ask to sign in again and the next save replaces the file.
    #[cfg(windows)]
    fn decrypt(protected: &str) -> Self {
        let decrypted = hex::decode(protected)
            .map_err(|error| Error::Other(format!("accounts.json is damaged: {error}")))
            .and_then(|blob| protect::unprotect(&blob))
            .and_then(|json| Ok(serde_json::from_slice(&json)?));
        decrypted.unwrap_or_else(|error| {
            tracing::warn!("account tokens are unusable, sign in again: {error}");
            Self::default()
        })
    }

    #[cfg(not(windows))]
    fn decrypt(_protected: &str) -> Self {
        tracing::warn!("accounts.json is encrypted with Windows DPAPI and cannot be read here");
        Self::default()
    }

    fn save(&self, paths: &Paths) -> Result<()> {
        #[cfg(windows)]
        {
            let blob = protect::protect(&serde_json::to_vec(self)?)?;
            write_json_private(&paths.accounts_file(), &serde_json::json!({ "protected": hex::encode(blob) }))
        }
        #[cfg(not(windows))]
        write_json_private(&paths.accounts_file(), self)
    }

    fn update<T>(paths: &Paths, change: impl FnOnce(&mut Self) -> T) -> Result<T> {
        let _guard = UPDATE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut file = Self::load(paths)?;
        let result = change(&mut file);
        file.save(paths)?;
        Ok(result)
    }
}

/// Rewrites an `accounts.json` left unencrypted by an older version. Meant to run once at startup.
pub fn protect_stored_tokens(paths: &Paths) -> Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    let _guard = UPDATE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (file, plain) = SecretsFile::load_stored(paths)?;
    if plain { file.save(paths) } else { Ok(()) }
}

/// Stores the tokens and adds the account to the public list; an account that is already there
/// (same kind and profile) is updated instead of duplicated. The first account becomes the selected one.
pub fn save_account(paths: &Paths, secrets: Secrets) -> Result<Account> {
    let account = secrets.public();
    SecretsFile::update(paths, |file| file.accounts.insert(account.id.clone(), secrets))?;
    GlobalSettings::update(paths, |settings| {
        match settings.accounts.iter_mut().find(|existing| existing.id == account.id) {
            Some(existing) => existing.name.clone_from(&account.name),
            None => settings.accounts.push(account.clone()),
        }
        settings.selected_account_id.get_or_insert_with(|| account.id.clone());
    })?;
    Ok(account)
}

/// Stores refreshed tokens of an account that is still there; a sign-out during the refresh must not bring it back.
fn replace_secrets(paths: &Paths, account_id: &str, secrets: Secrets) -> Result<()> {
    SecretsFile::update(paths, |file| {
        if let Some(stored) = file.accounts.get_mut(account_id) {
            *stored = secrets;
        }
    })
}

/// Removes the account and its tokens; the selection moves to the first remaining account.
pub fn remove_account(paths: &Paths, account_id: &str) -> Result<()> {
    GlobalSettings::update(paths, |settings| {
        settings.accounts.retain(|account| account.id != account_id);
        if settings.selected_account().is_none() {
            settings.selected_account_id = settings.accounts.first().map(|account| account.id.clone());
        }
    })?;
    SecretsFile::update(paths, |file| file.accounts.remove(account_id)).map(|_| ())
}

/// Finishes a Microsoft sign-in started with [`microsoft::request_device_code`]: waits for the user
/// to enter the code, then gets the Minecraft profile and stores the account.
pub async fn complete_microsoft_login(
    paths: &Paths,
    http: &reqwest::Client,
    client_id: &str,
    device: &DeviceCode,
) -> Result<Account> {
    let tokens = microsoft::poll_token(http, client_id, device).await?;
    let secrets = microsoft::secrets_from_tokens(http, tokens, None).await?;
    save_account(paths, secrets)
}

/// Result of [`elyby_login`].
pub enum ElybyOutcome {
    Done(Account),
    TwoFactorRequired,
}

pub async fn elyby_login(
    paths: &Paths,
    http: &reqwest::Client,
    username: &str,
    password: &str,
    totp: Option<&str>,
) -> Result<ElybyOutcome> {
    match elyby::authenticate(http, username, password, totp).await? {
        ElybyLogin::Done(secrets) => save_account(paths, secrets).map(ElybyOutcome::Done),
        ElybyLogin::TwoFactorRequired => Ok(ElybyOutcome::TwoFactorRequired),
    }
}

/// What a launch needs for an account.
#[derive(Debug)]
pub struct LaunchIdentity {
    pub account: GameAccount,
    /// `-javaagent` arguments (authlib-injector for ely.by).
    pub java_agent_args: Vec<String>,
}

/// Turns a stored account into the game identity, refreshing its tokens when they are about to expire.
/// `msa_client_id` is the build's Azure application id, `None` in builds without one.
pub async fn resolve_launch_identity(
    paths: &Paths,
    http: &reqwest::Client,
    msa_client_id: Option<&str>,
    account: &Account,
) -> Result<LaunchIdentity> {
    match account.kind {
        AccountKind::Offline => Ok(LaunchIdentity { account: GameAccount::offline(&account.name), java_agent_args: Vec::new() }),
        AccountKind::Microsoft => {
            let secrets = fresh_microsoft_secrets(paths, http, msa_client_id, &account.id).await?;
            let game = GameAccount::authenticated(&secrets.name, &secrets.uuid, &secrets.access_token, "msa");
            Ok(LaunchIdentity { account: game, java_agent_args: Vec::new() })
        }
        AccountKind::Elyby => {
            let secrets = valid_elyby_secrets(paths, http, &account.id).await?;
            let jar = elyby::ensure_injector(paths, http).await?;
            let game = GameAccount::authenticated(&secrets.name, &secrets.uuid, &secrets.access_token, "mojang");
            Ok(LaunchIdentity { account: game, java_agent_args: vec![elyby::java_agent_arg(&jar)] })
        }
    }
}

fn stored_secrets(paths: &Paths, account_id: &str, relogin: &str) -> Result<Secrets> {
    SecretsFile::load(paths)?.accounts.remove(account_id).ok_or_else(|| Error::Other(relogin.to_owned()))
}

async fn fresh_microsoft_secrets(
    paths: &Paths,
    http: &reqwest::Client,
    msa_client_id: Option<&str>,
    account_id: &str,
) -> Result<MicrosoftSecrets> {
    let load = || match stored_secrets(paths, account_id, microsoft::RELOGIN_MESSAGE)? {
        Secrets::Microsoft(secrets) => Ok(secrets),
        Secrets::Elyby(_) => Err(Error::Other(microsoft::RELOGIN_MESSAGE.to_owned())),
    };
    let now = chrono::Utc::now().timestamp();
    let current = load()?;
    if !microsoft::needs_refresh(current.expires_at, now) {
        return Ok(current);
    }

    let _guard = REFRESH_LOCK.lock().await;
    // Another launch may have refreshed the token while this one waited for the lock.
    let current = load()?;
    if !microsoft::needs_refresh(current.expires_at, now) {
        return Ok(current);
    }
    let client_id = msa_client_id.ok_or_else(|| Error::Other(NO_MSA_CLIENT_ID.to_owned()))?;
    let refreshed = async {
        let tokens = microsoft::refresh(http, client_id, &current.refresh_token).await?;
        if let Some(rotated) = &tokens.refresh_token {
            // The old refresh token is spent now, so the new one is saved even if a later step fails.
            let rotated = MicrosoftSecrets { refresh_token: rotated.clone(), ..current.clone() };
            replace_secrets(paths, account_id, Secrets::Microsoft(rotated))?;
        }
        microsoft::secrets_from_tokens(http, tokens, Some(&current.refresh_token)).await
    }
    .await;
    let secrets = match refreshed {
        Ok(Secrets::Microsoft(secrets)) => secrets,
        Ok(Secrets::Elyby(_)) => unreachable!("a Microsoft refresh yields Microsoft secrets"),
        // A token that has not expired yet is still good for the launch, unless Microsoft says to sign in again.
        Err(error) if current.expires_at > now && !microsoft::requires_relogin(&error) => {
            tracing::warn!("cannot refresh the Microsoft token, using the stored one: {error}");
            return Ok(current);
        }
        Err(error) => return Err(error),
    };
    replace_secrets(paths, account_id, Secrets::Microsoft(secrets.clone()))?;
    if secrets.name != current.name {
        GlobalSettings::update(paths, |settings| {
            if let Some(account) = settings.accounts.iter_mut().find(|account| account.id == account_id) {
                account.name.clone_from(&secrets.name);
            }
        })?;
    }
    Ok(secrets)
}

async fn valid_elyby_secrets(paths: &Paths, http: &reqwest::Client, account_id: &str) -> Result<ElybySecrets> {
    // Two launches must not refresh the same token: the loser would be told to sign in again. The secrets
    // are read after the lock is taken, so a launch that waited sees the winner's new token.
    let _guard = REFRESH_LOCK.lock().await;
    let Secrets::Elyby(secrets) = stored_secrets(paths, account_id, elyby::RELOGIN_MESSAGE)? else {
        return Err(Error::Other(elyby::RELOGIN_MESSAGE.to_owned()));
    };
    match elyby::ensure_valid(http, &secrets).await? {
        Some(refreshed) => {
            replace_secrets(paths, account_id, Secrets::Elyby(refreshed.clone()))?;
            Ok(refreshed)
        }
        None => Ok(secrets),
    }
}

pub const NO_MSA_CLIENT_ID: &str = "Microsoft-вход недоступен: сборка без K0T1K_MSA_CLIENT_ID";

/// Sends a JSON request and parses the JSON answer; `service` names the server in error messages.
pub(crate) async fn send_json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
    url: &str,
    service: &str,
) -> Result<T> {
    let response = request.send().await.map_err(|source| http_error(url, source))?;
    if !response.status().is_success() {
        return Err(Error::Other(format!("{service} вернул ошибку {}", response.status())));
    }
    response.json().await.map_err(|source| http_error(url, source))
}

pub(crate) fn http_error(url: &str, source: reqwest::Error) -> Error {
    Error::Http { url: url.to_owned(), source }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn microsoft_secrets(uuid: &str) -> Secrets {
        Secrets::Microsoft(MicrosoftSecrets {
            uuid: uuid.to_owned(),
            name: "Steve".to_owned(),
            refresh_token: "refresh-secret".to_owned(),
            access_token: "access-secret".to_owned(),
            expires_at: 0,
        })
    }

    #[test]
    fn debug_output_hides_tokens() {
        let text = format!("{:?}", microsoft_secrets("abc"));
        assert!(text.contains("ms-abc"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn save_updates_instead_of_duplicating_and_remove_deletes_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let first = save_account(&paths, microsoft_secrets("abc")).unwrap();
        assert_eq!(first.id, "ms-abc");
        let Secrets::Microsoft(mut renamed) = microsoft_secrets("abc") else { unreachable!() };
        renamed.name = "Alex".to_owned();
        save_account(&paths, Secrets::Microsoft(renamed)).unwrap();
        save_account(
            &paths,
            Secrets::Elyby(ElybySecrets {
                uuid: "abc".to_owned(),
                name: "Steve".to_owned(),
                access_token: "t".to_owned(),
                client_token: "c".to_owned(),
            }),
        )
        .unwrap();

        let settings = GlobalSettings::load(&paths).unwrap();
        assert_eq!(settings.accounts.len(), 2);
        assert_eq!(settings.accounts[0].name, "Alex");
        assert_eq!(settings.selected_account_id.as_deref(), Some("ms-abc"));
        let settings_json = std::fs::read_to_string(paths.settings_file()).unwrap();
        assert!(!settings_json.contains("secret"));
        assert_eq!(SecretsFile::load(&paths).unwrap().accounts.len(), 2);

        remove_account(&paths, "ms-abc").unwrap();
        assert_eq!(GlobalSettings::load(&paths).unwrap().selected_account_id.as_deref(), Some("elyby-abc"));
        assert_eq!(SecretsFile::load(&paths).unwrap().accounts.keys().collect::<Vec<_>>(), ["elyby-abc"]);
    }

    #[test]
    fn refreshed_tokens_do_not_resurrect_a_removed_account() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, microsoft_secrets("abc")).unwrap();
        remove_account(&paths, "ms-abc").unwrap();
        replace_secrets(&paths, "ms-abc", microsoft_secrets("abc")).unwrap();
        assert!(SecretsFile::load(&paths).unwrap().accounts.is_empty());
    }

    #[test]
    fn tokens_are_not_stored_in_plain_text_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, microsoft_secrets("abc")).unwrap();
        let raw = std::fs::read_to_string(paths.accounts_file()).unwrap();
        assert_eq!(raw.contains("refresh-secret"), !cfg!(windows));
        assert_eq!(SecretsFile::load(&paths).unwrap().accounts.keys().collect::<Vec<_>>(), ["ms-abc"]);
    }

    #[test]
    fn plain_file_from_an_older_version_is_read_and_protected() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut file = SecretsFile::default();
        file.accounts.insert("ms-abc".to_owned(), microsoft_secrets("abc"));
        std::fs::write(paths.accounts_file(), serde_json::to_vec(&file).unwrap()).unwrap();
        protect_stored_tokens(&paths).unwrap();
        let (loaded, plain) = SecretsFile::load_stored(&paths).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(plain, !cfg!(windows));
    }

    #[test]
    fn unreadable_encrypted_file_means_signing_in_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        std::fs::write(paths.accounts_file(), r#"{"protected":"00ff"}"#).unwrap();
        assert!(SecretsFile::load(&paths).unwrap().accounts.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn accounts_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, microsoft_secrets("abc")).unwrap();
        let mode = std::fs::metadata(paths.accounts_file()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn offline_identity_needs_no_network() {
        let dir = tempfile::tempdir().unwrap();
        let account = Account { id: "x".to_owned(), name: "Notch".to_owned(), kind: AccountKind::Offline };
        let identity = resolve_launch_identity(&Paths::new(dir.path()), &reqwest::Client::new(), None, &account).await.unwrap();
        assert_eq!(identity.account.user_type, "offline");
        assert!(identity.java_agent_args.is_empty());
    }

    #[tokio::test]
    async fn valid_microsoft_token_is_used_without_refresh_and_missing_secrets_ask_to_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let account = Account { id: "ms-abc".to_owned(), name: "Steve".to_owned(), kind: AccountKind::Microsoft };
        let http = reqwest::Client::new();
        assert!(resolve_launch_identity(&paths, &http, None, &account).await.is_err());

        let Secrets::Microsoft(mut secrets) = microsoft_secrets("abc") else { unreachable!() };
        secrets.expires_at = chrono::Utc::now().timestamp() + 3600;
        save_account(&paths, Secrets::Microsoft(secrets)).unwrap();
        let identity = resolve_launch_identity(&paths, &http, None, &account).await.unwrap();
        assert_eq!(identity.account.session, "token:access-secret:abc");
        assert_eq!(identity.account.user_type, "msa");
    }

    #[tokio::test]
    async fn expired_microsoft_token_without_client_id_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        save_account(&paths, microsoft_secrets("abc")).unwrap();
        let account = Account { id: "ms-abc".to_owned(), name: "Steve".to_owned(), kind: AccountKind::Microsoft };
        let error = resolve_launch_identity(&paths, &reqwest::Client::new(), None, &account).await.unwrap_err();
        assert!(error.to_string().contains("K0T1K_MSA_CLIENT_ID"));
    }
}
