//! Microsoft sign-in: OAuth device code flow, then Xbox Live, XSTS and Minecraft services.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Secrets, http_error, send_json};
use crate::error::{Error, Result};

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SCOPE: &str = "XboxLive.signin offline_access";

/// Added to the polling interval when Microsoft answers `slow_down`.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// Refresh the Minecraft token when it expires within this many seconds.
const REFRESH_MARGIN_SECS: i64 = 5 * 60;

pub const RELOGIN_MESSAGE: &str = "Сессия Microsoft истекла: войдите в аккаунт заново";
const CLIENT_NOT_APPROVED_MESSAGE: &str = "Minecraft отказал во входе (403): идентификатор приложения лаунчера не одобрен для Minecraft API (вероятно, ошибка конфигурации сборки)";
const NO_PROFILE_MESSAGE: &str = "У аккаунта нет Minecraft или ещё не создан профиль (ник) — зайдите на minecraft.net";
const EXPIRED_CODE_MESSAGE: &str = "Код входа Microsoft истёк: начните вход заново";

/// The code the user types at `verification_uri`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Result of one request to the token endpoint.
enum Poll {
    Done(TokenResponse),
    Pending,
    SlowDown,
}

/// Microsoft tokens; `refresh_token` is `None` when Microsoft did not rotate it.
pub struct MicrosoftTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

pub async fn request_device_code(http: &reqwest::Client, client_id: &str) -> Result<DeviceCode> {
    let request = http.post(DEVICE_CODE_URL).form(&[("client_id", client_id), ("scope", SCOPE)]);
    let response = request.send().await.map_err(|source| http_error(DEVICE_CODE_URL, source))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Other(format!("Microsoft не выдал код входа: {}", oauth_message(&body))));
    }
    response.json().await.map_err(|source| http_error(DEVICE_CODE_URL, source))
}

/// Polls until the user finishes (or refuses) the sign-in. Dropping the future cancels it.
pub async fn poll_token(http: &reqwest::Client, client_id: &str, device: &DeviceCode) -> Result<MicrosoftTokens> {
    let mut interval = Duration::from_secs(device.interval.max(1));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Other(EXPIRED_CODE_MESSAGE.to_owned()));
        }
        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", device.device_code.as_str()),
        ];
        match token_request(http, &form).await {
            Ok(Poll::Done(tokens)) => return Ok(into_tokens(tokens)),
            Ok(Poll::Pending) => {}
            Ok(Poll::SlowDown) => interval += SLOW_DOWN_STEP,
            // A network hiccup must not end a sign-in the user is still completing; the deadline does.
            Err(Error::Http { url, source }) => tracing::warn!("device code poll failed for {url}, retrying: {source}"),
            Err(error) => return Err(error),
        }
    }
}

/// Exchanges the refresh token for new Microsoft tokens; `invalid_grant` means the user must sign in again.
pub async fn refresh(http: &reqwest::Client, client_id: &str, refresh_token: &str) -> Result<MicrosoftTokens> {
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("scope", SCOPE),
    ];
    match token_request(http, &form).await? {
        Poll::Done(tokens) => Ok(into_tokens(tokens)),
        Poll::Pending | Poll::SlowDown => Err(Error::Other(RELOGIN_MESSAGE.to_owned())),
    }
}

fn into_tokens(tokens: TokenResponse) -> MicrosoftTokens {
    MicrosoftTokens { access_token: tokens.access_token, refresh_token: tokens.refresh_token }
}

async fn token_request(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<Poll> {
    let response = http.post(TOKEN_URL).form(form).send().await.map_err(|source| http_error(TOKEN_URL, source))?;
    if response.status().is_success() {
        return Ok(Poll::Done(response.json().await.map_err(|source| http_error(TOKEN_URL, source))?));
    }
    let body = response.text().await.unwrap_or_default();
    classify_token_error(&body)
}

/// Maps an error body of the token endpoint to "keep waiting" or a message for the user.
fn classify_token_error(body: &str) -> Result<Poll> {
    let Ok(error) = serde_json::from_str::<OAuthError>(body) else {
        return Err(Error::Other(format!("Microsoft вернул непонятный ответ: {}", body.chars().take(200).collect::<String>())));
    };
    match error.error.as_str() {
        "authorization_pending" => Ok(Poll::Pending),
        "slow_down" => Ok(Poll::SlowDown),
        "expired_token" => Err(Error::Other(EXPIRED_CODE_MESSAGE.to_owned())),
        "authorization_declined" => Err(Error::Other("Вход в Microsoft отклонён".to_owned())),
        "invalid_grant" => Err(Error::Other(RELOGIN_MESSAGE.to_owned())),
        _ => Err(Error::Other(format!("Ошибка входа Microsoft: {}", first_line(&error)))),
    }
}

fn first_line(error: &OAuthError) -> &str {
    error.error_description.lines().next().filter(|line| !line.is_empty()).unwrap_or(&error.error)
}

fn oauth_message(body: &str) -> String {
    match serde_json::from_str::<OAuthError>(body) {
        Ok(error) => first_line(&error).to_owned(),
        Err(_) => body.chars().take(200).collect(),
    }
}

#[derive(Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize)]
struct XboxError {
    #[serde(rename = "XErr")]
    x_err: u64,
}

#[derive(Deserialize)]
struct MinecraftLogin {
    access_token: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

/// Microsoft tokens to a Minecraft session (Xbox Live, XSTS, Minecraft services) as stored secrets.
/// `previous_refresh_token` is kept when Microsoft did not rotate it.
pub async fn secrets_from_tokens(
    http: &reqwest::Client,
    tokens: MicrosoftTokens,
    previous_refresh_token: Option<&str>,
) -> Result<Secrets> {
    let refresh_token = tokens
        .refresh_token
        .or_else(|| previous_refresh_token.map(str::to_owned))
        .ok_or_else(|| Error::Other("Microsoft не вернул токен обновления".to_owned()))?;

    let xbl_body = json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={}", tokens.access_token),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let xbl: XboxResponse = send_json(http.post(XBL_URL).json(&xbl_body), XBL_URL, "Xbox Live").await?;
    let uhs = xbl
        .display_claims
        .xui
        .first()
        .ok_or_else(|| Error::Other("Xbox Live не вернул данные аккаунта".to_owned()))?
        .uhs
        .clone();

    let xsts_body = json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });
    let response = http.post(XSTS_URL).json(&xsts_body).send().await.map_err(|source| http_error(XSTS_URL, source))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let body = response.text().await.unwrap_or_default();
        let code = serde_json::from_str::<XboxError>(&body).map_or(0, |error| error.x_err);
        return Err(Error::Other(xsts_error_message(code)));
    }
    if !response.status().is_success() {
        return Err(Error::Other(format!("Xbox Live (XSTS) вернул ошибку {}", response.status())));
    }
    let xsts: XboxResponse = response.json().await.map_err(|source| http_error(XSTS_URL, source))?;

    let login_body = json!({ "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token) });
    let response = http
        .post(MINECRAFT_LOGIN_URL)
        .json(&login_body)
        .send()
        .await
        .map_err(|source| http_error(MINECRAFT_LOGIN_URL, source))?;
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(Error::Other(CLIENT_NOT_APPROVED_MESSAGE.to_owned()));
    }
    if !response.status().is_success() {
        return Err(Error::Other(format!("Minecraft вернул ошибку {}", response.status())));
    }
    let login: MinecraftLogin = response.json().await.map_err(|source| http_error(MINECRAFT_LOGIN_URL, source))?;

    let response = http
        .get(MINECRAFT_PROFILE_URL)
        .bearer_auth(&login.access_token)
        .send()
        .await
        .map_err(|source| http_error(MINECRAFT_PROFILE_URL, source))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(Error::Other(NO_PROFILE_MESSAGE.to_owned()));
    }
    if !response.status().is_success() {
        return Err(Error::Other(format!("Профиль Minecraft недоступен: ошибка {}", response.status())));
    }
    let profile: MinecraftProfile = response.json().await.map_err(|source| http_error(MINECRAFT_PROFILE_URL, source))?;

    Ok(Secrets::Microsoft(MicrosoftSecrets {
        uuid: profile.id,
        name: profile.name,
        refresh_token,
        access_token: login.access_token,
        expires_at: chrono::Utc::now().timestamp() + login.expires_in,
    }))
}

/// Explains an XSTS `XErr` code.
fn xsts_error_message(code: u64) -> String {
    match code {
        2148916233 => "У этого аккаунта Microsoft нет учётной записи Xbox: создайте её на xbox.com".to_owned(),
        2148916235 => "Xbox Live недоступен в стране этого аккаунта".to_owned(),
        2148916236 | 2148916237 => "Аккаунту нужна проверка возраста (adult verification) на сайте Xbox".to_owned(),
        2148916238 => "Это детский аккаунт: взрослый должен добавить его в семейную группу Microsoft".to_owned(),
        _ => format!("Xbox Live отказал в доступе (XErr {code})"),
    }
}

/// Whether a refresh failure is final (the refresh token is dead or the Xbox account is unusable), as opposed
/// to a transient one after which a still valid Minecraft token can be used.
pub fn requires_relogin(error: &Error) -> bool {
    let Error::Other(message) = error else { return false };
    message == RELOGIN_MESSAGE
        || message.starts_with("Xbox Live отказал в доступе")
        || [2148916233, 2148916235, 2148916236, 2148916237, 2148916238].iter().any(|&code| *message == xsts_error_message(code))
}

/// Whether the Minecraft token (valid until `expires_at`, unix seconds) is too close to expiry to launch with.
pub fn needs_refresh(expires_at: i64, now: i64) -> bool {
    expires_at - now < REFRESH_MARGIN_SECS
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrosoftSecrets {
    pub uuid: String,
    pub name: String,
    pub refresh_token: String,
    pub access_token: String,
    /// Unix seconds.
    pub expires_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsts_codes_have_messages() {
        assert!(xsts_error_message(2148916233).contains("Xbox"));
        assert!(xsts_error_message(2148916235).contains("стран"));
        assert_eq!(xsts_error_message(2148916236), xsts_error_message(2148916237));
        assert!(xsts_error_message(2148916238).contains("семейн"));
        assert!(xsts_error_message(1).contains("XErr 1"));
    }

    #[test]
    fn only_final_refresh_failures_require_a_new_sign_in() {
        assert!(requires_relogin(&Error::Other(RELOGIN_MESSAGE.to_owned())));
        assert!(requires_relogin(&Error::Other(xsts_error_message(2148916233))));
        assert!(requires_relogin(&Error::Other(xsts_error_message(1))));
        assert!(!requires_relogin(&Error::Other("Minecraft вернул ошибку 503".to_owned())));
    }

    #[test]
    fn refresh_decision_uses_a_five_minute_margin() {
        assert!(!needs_refresh(1_000 + 301, 1_000));
        assert!(needs_refresh(1_000 + 299, 1_000));
        assert!(needs_refresh(900, 1_000));
    }

    #[test]
    fn token_errors_are_classified() {
        let pending = r#"{"error":"authorization_pending","error_description":"AADSTS70016: pending"}"#;
        assert!(matches!(classify_token_error(pending), Ok(Poll::Pending)));
        assert!(matches!(classify_token_error(r#"{"error":"slow_down"}"#), Ok(Poll::SlowDown)));
        for (body, expected) in [
            (r#"{"error":"expired_token"}"#, "истёк"),
            (r#"{"error":"authorization_declined"}"#, "отклонён"),
            (r#"{"error":"invalid_grant","error_description":"expired"}"#, "заново"),
        ] {
            let Err(Error::Other(message)) = classify_token_error(body) else { panic!("{body}") };
            assert!(message.contains(expected), "{message}");
        }
        assert!(classify_token_error("<html>").is_err());
    }

    #[test]
    fn responses_parse() {
        let device: DeviceCode = serde_json::from_str(
            r#"{"user_code":"ABCD1234","device_code":"dc","verification_uri":"https://www.microsoft.com/link","expires_in":900,"interval":5,"message":"x"}"#,
        )
        .unwrap();
        assert_eq!((device.user_code.as_str(), device.expires_in, device.interval), ("ABCD1234", 900, 5));
        let xbl: XboxResponse =
            serde_json::from_str(r#"{"IssueInstant":"x","Token":"t","DisplayClaims":{"xui":[{"uhs":"12345"}]}}"#).unwrap();
        assert_eq!((xbl.token.as_str(), xbl.display_claims.xui[0].uhs.as_str()), ("t", "12345"));
        let error: XboxError = serde_json::from_str(r#"{"Identity":"0","XErr":2148916233,"Message":""}"#).unwrap();
        assert_eq!(error.x_err, 2148916233);
        let login: MinecraftLogin = serde_json::from_str(r#"{"username":"u","access_token":"a","expires_in":86400}"#).unwrap();
        assert_eq!(login.expires_in, 86400);
    }
}
