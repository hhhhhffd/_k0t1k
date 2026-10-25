//! ely.by accounts: the Yggdrasil API of authserver.ely.by and the authlib-injector agent that points
//! the game at ely.by's session server.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Secrets, http_error};
use crate::download::{Checksum, download_checked};
use crate::error::{Error, Result};
use crate::i18n::tr;
use crate::paths::Paths;
use crate::tr_fmt;

const AUTH_URL: &str = "https://authserver.ely.by/auth";
const INJECTOR_HOST: &str = "authlib-injector.yushi.moe";
const INJECTOR_LATEST_URL: &str = "https://authlib-injector.yushi.moe/artifact/latest.json";

pub fn relogin_message() -> &'static str {
    tr("The ely.by session has expired: sign in again", "Сессия ely.by истекла: войдите в аккаунт заново")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    access_token: String,
    selected_profile: Option<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiError {
    #[serde(default)]
    error_message: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElybySecrets {
    pub uuid: String,
    pub name: String,
    pub access_token: String,
    pub client_token: String,
}

/// Outcome of a password sign-in.
pub enum Login {
    Done(Secrets),
    /// The account has two-factor auth: ask for the code and call again with it.
    TwoFactorRequired,
}

/// Signs in with a login (email or nickname) and password; `totp` is the two-factor code, if any.
/// The password is only sent to ely.by and never stored.
pub async fn authenticate(http: &reqwest::Client, username: &str, password: &str, totp: Option<&str>) -> Result<Login> {
    let url = format!("{AUTH_URL}/authenticate");
    let client_token = uuid::Uuid::new_v4().simple().to_string();
    let password = match totp {
        Some(code) => format!("{password}:{code}"),
        None => password.to_owned(),
    };
    let body = json!({ "username": username, "password": password, "clientToken": client_token, "requestUser": true });
    let response = http.post(&url).json(&body).send().await.map_err(|source| http_error(&url, source))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return match classify_error(status, &text, totp.is_some()) {
            Failure::TwoFactor => Ok(Login::TwoFactorRequired),
            Failure::Message(message) => Err(Error::Other(message)),
        };
    }
    let auth: AuthResponse = response.json().await.map_err(|source| http_error(&url, source))?;
    let profile = auth.selected_profile.ok_or_else(|| Error::Other(tr("This ely.by account has no player profile", "На аккаунте ely.by нет профиля игрока").to_owned()))?;
    Ok(Login::Done(Secrets::Elyby(ElybySecrets {
        uuid: profile.id,
        name: profile.name,
        access_token: auth.access_token,
        client_token,
    })))
}

enum Failure {
    TwoFactor,
    Message(String),
}

fn classify_error(status: reqwest::StatusCode, body: &str, sent_totp: bool) -> Failure {
    let message = serde_json::from_str::<ApiError>(body).map(|error| error.error_message).unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    if lower.contains("two factor") || lower.contains("two-factor") {
        // A second "protected" answer after the code was sent means the code was not accepted.
        return if sent_totp { Failure::Message(tr("Wrong two-factor code", "Неверный код двухфакторной защиты").to_owned()) } else { Failure::TwoFactor };
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let text = if lower.contains("invalid credentials") || lower.contains("password") {
            tr("Wrong ely.by login or password", "Неверный логин или пароль ely.by").to_owned()
        } else if lower.contains("temporarily") || lower.contains("too many") {
            tr("ely.by is temporarily blocking sign-in: too many attempts, wait a bit", "ely.by временно блокирует вход: слишком много попыток, подождите").to_owned()
        } else if lower.contains("banned") {
            tr("This ely.by account is banned", "Аккаунт ely.by заблокирован").to_owned()
        } else {
            tr_fmt!("ely.by refused the sign-in: {message}", "ely.by отказал во входе: {message}")
        };
        return Failure::Message(text);
    }
    Failure::Message(tr_fmt!("ely.by returned error {status}", "ely.by вернул ошибку {status}"))
}

/// Makes sure the stored access token works before a launch: validates it and refreshes it when
/// rejected. `Ok(None)` means the token is fine; a network failure keeps the stored token, since
/// offline play does not need it.
pub async fn ensure_valid(http: &reqwest::Client, secrets: &ElybySecrets) -> Result<Option<ElybySecrets>> {
    let url = format!("{AUTH_URL}/validate");
    let response = match http.post(&url).json(&json!({ "accessToken": secrets.access_token })).send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!("cannot validate the ely.by token, using the stored one: {error}");
            return Ok(None);
        }
    };
    if response.status().is_success() {
        return Ok(None);
    }
    if !matches!(response.status(), reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN) {
        return Err(Error::Other(tr_fmt!("ely.by returned error {} while checking the token", "ely.by вернул ошибку {} при проверке токена", response.status())));
    }

    let url = format!("{AUTH_URL}/refresh");
    let body = json!({ "accessToken": secrets.access_token, "clientToken": secrets.client_token, "requestUser": true });
    let response = http.post(&url).json(&body).send().await.map_err(|source| http_error(&url, source))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(Error::Other(relogin_message().to_owned()));
    }
    if !status.is_success() {
        return Err(Error::Other(tr_fmt!("ely.by returned error {status} while refreshing the token", "ely.by вернул ошибку {status} при обновлении токена")));
    }
    let refreshed: AuthResponse = response.json().await.map_err(|source| http_error(&url, source))?;
    Ok(Some(ElybySecrets { access_token: refreshed.access_token, ..secrets.clone() }))
}

#[derive(Deserialize)]
struct InjectorRelease {
    version: String,
    download_url: String,
    checksums: InjectorChecksums,
}

#[derive(Deserialize)]
struct InjectorChecksums {
    sha256: String,
}

fn injector_dir(paths: &Paths) -> PathBuf {
    paths.libraries_dir().join("moe").join("yushi").join("authlib-injector")
}

/// Path of the cached authlib-injector jar, downloading the latest release first. Without a network
/// the newest jar already on disk is used.
pub async fn ensure_injector(paths: &Paths, http: &reqwest::Client) -> Result<PathBuf> {
    match latest_release(http).await {
        Ok(release) => {
            let dir = injector_dir(paths).join(&release.version);
            let jar = dir.join(format!("authlib-injector-{}.jar", release.version));
            let checksum = Checksum::Sha256(release.checksums.sha256.to_ascii_lowercase());
            match download_checked(http, &release.download_url, &jar, Some(&checksum)).await {
                Ok(()) => Ok(jar),
                Err(error) => cached_injector(&injector_dir(paths)).ok_or(error),
            }
        }
        Err(error) => cached_injector(&injector_dir(paths)).ok_or(error),
    }
}

async fn latest_release(http: &reqwest::Client) -> Result<InjectorRelease> {
    let release: InjectorRelease = crate::download::get_json(http, INJECTOR_LATEST_URL).await?;
    // The version becomes a folder name and the URL is fetched as is, so both are checked.
    let version_ok = !release.version.is_empty()
        && release.version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    let host_ok = reqwest::Url::parse(&release.download_url)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some(INJECTOR_HOST));
    if !version_ok || !host_ok {
        return Err(Error::Other("authlib-injector: unexpected response from the update server".to_owned()));
    }
    Ok(release)
}

/// The jar of the highest version folder under `dir`, if any.
fn cached_injector(dir: &Path) -> Option<PathBuf> {
    let mut versions: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(|entry| entry.ok()).collect();
    versions.sort_by_key(|entry| version_key(&entry.file_name().to_string_lossy()));
    versions.iter().rev().find_map(|entry| {
        let version = entry.file_name().to_string_lossy().into_owned();
        let jar = entry.path().join(format!("authlib-injector-{version}.jar"));
        jar.is_file().then_some(jar)
    })
}

/// Orders versions numerically per dot-separated part (1.2.10 after 1.2.9).
fn version_key(version: &str) -> Vec<u64> {
    version.split(['.', '-']).map(|part| part.parse().unwrap_or(0)).collect()
}

/// `-javaagent` argument that points the game at ely.by.
pub fn java_agent_arg(jar: &Path) -> String {
    format!("-javaagent:{}=ely.by", jar.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    fn message(failure: Failure) -> String {
        match failure {
            Failure::Message(message) => message,
            Failure::TwoFactor => "two-factor".to_owned(),
        }
    }

    #[test]
    fn two_factor_is_detected() {
        let body = r#"{"error":"ForbiddenOperationException","errorMessage":"Account protected with two factor auth."}"#;
        assert!(matches!(classify_error(StatusCode::UNAUTHORIZED, body, false), Failure::TwoFactor));
        assert!(message(classify_error(StatusCode::UNAUTHORIZED, body, true)).contains("code"));
    }

    #[test]
    fn credentials_errors_are_explained() {
        let body = r#"{"error":"ForbiddenOperationException","errorMessage":"Invalid credentials. Invalid nickname or password."}"#;
        assert!(message(classify_error(StatusCode::UNAUTHORIZED, body, false)).contains("Wrong ely.by login"));
        assert!(message(classify_error(StatusCode::BAD_GATEWAY, "<html>", false)).contains("502"));
    }

    #[test]
    fn authenticate_response_parses() {
        let json = r#"{"accessToken":"a","clientToken":"c","selectedProfile":{"id":"0123456789abcdef0123456789abcdef","name":"Steve"},"user":{"id":"1"}}"#;
        let response: AuthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.selected_profile.unwrap().name, "Steve");
    }

    #[test]
    fn injector_release_parses() {
        let json = r#"{"build_no":57,"version":"1.2.5","release_time":"x","download_url":"https://example.org/a.jar","checksums":{"sha256":"ab"}}"#;
        let release: InjectorRelease = serde_json::from_str(json).unwrap();
        assert_eq!((release.version.as_str(), release.checksums.sha256.as_str()), ("1.2.5", "ab"));
    }

    #[test]
    fn newest_cached_injector_is_used() {
        let dir = tempfile::tempdir().unwrap();
        for version in ["1.2.9", "1.2.10", "1.2.11"] {
            let folder = dir.path().join(version);
            std::fs::create_dir_all(&folder).unwrap();
            // 1.2.11 has no jar, so the next best one wins.
            if version != "1.2.11" {
                std::fs::write(folder.join(format!("authlib-injector-{version}.jar")), b"jar").unwrap();
            }
        }
        let jar = cached_injector(dir.path()).unwrap();
        assert!(jar.ends_with("1.2.10/authlib-injector-1.2.10.jar"));
        assert!(cached_injector(&dir.path().join("missing")).is_none());
    }
}
