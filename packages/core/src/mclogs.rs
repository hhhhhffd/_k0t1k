//! Uploading a game log to mclo.gs (<https://api.mclo.gs/>). Nothing here runs unless the user asks for it.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const UPLOAD_URL: &str = "https://api.mclo.gs/1/log";
/// mclo.gs limits (`/1/limits`); older content is cut away on our side so the crash report at the end survives.
const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_LINES: usize = 25_000;
const SEPARATOR: &str = "\n\n===== Crash report =====\n\n";
/// Secrets shorter than this would mask ordinary text (an offline account's placeholder token).
const MIN_SECRET_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UploadedLog {
    pub id: String,
    pub url: String,
    pub raw: String,
}

#[derive(Deserialize)]
struct Response {
    success: bool,
    id: Option<String>,
    url: Option<String>,
    raw: Option<String>,
    error: Option<String>,
}

fn parse_response(body: &str) -> Result<UploadedLog> {
    let response: Response = serde_json::from_str(body)?;
    match response {
        Response { success: true, id: Some(id), url: Some(url), raw: Some(raw), .. } => Ok(UploadedLog { id, url, raw }),
        Response { success: false, error, .. } => {
            Err(Error::Other(format!("mclo.gs rejected the log: {}", error.unwrap_or_else(|| "unknown error".to_owned()))))
        }
        _ => Err(Error::Other("mclo.gs sent an incomplete response".to_owned())),
    }
}

/// Masks the value after every `--accessToken` and every occurrence of the given secrets.
fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("--accessToken") {
        let after = start + "--accessToken".len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        let spaces = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        out.push_str(&tail[..spaces]);
        let value = &tail[spaces..];
        let value_len = value.find(|c: char| c.is_whitespace() || c == '"').unwrap_or(value.len());
        if value_len > 0 {
            out.push_str("***");
        }
        rest = &value[value_len..];
    }
    out.push_str(rest);
    mask_secrets(&out, secrets)
}

/// Replaces every occurrence of each secret with `***`; secrets that are too short are ignored.
pub fn mask_secrets<S: AsRef<str>>(text: &str, secrets: &[S]) -> String {
    let mut out = text.to_owned();
    for secret in secrets.iter().map(AsRef::as_ref).filter(|secret| secret.len() >= MIN_SECRET_LEN) {
        out = out.replace(secret, "***");
    }
    out
}

/// Keeps the newest part of `text` within the line and byte limits.
fn truncate_tail(text: &str) -> &str {
    let mut text = text;
    let lines = text.bytes().filter(|&byte| byte == b'\n').count() + 1;
    if lines > MAX_LINES {
        let skip = lines - MAX_LINES;
        let mut offset = 0;
        for _ in 0..skip {
            offset += text[offset..].find('\n').map_or(0, |at| at + 1);
        }
        text = &text[offset..];
    }
    if text.len() > MAX_BYTES {
        let mut start = text.len() - MAX_BYTES;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        // Begin at a line start unless the tail has none.
        if let Some(newline) = text[start..].find('\n') {
            start += newline + 1;
        }
        text = &text[start..];
    }
    text
}

/// The text to upload: the session's lines, then the crash report if there is one, with `secrets`
/// and access tokens masked and the oldest lines dropped when the limits are exceeded.
pub fn prepare_content(lines: &[&str], crash_report: Option<&str>, secrets: &[&str]) -> String {
    let mut text = lines.join("\n");
    if let Some(report) = crash_report {
        text.push_str(SEPARATOR);
        text.push_str(report);
    }
    truncate_tail(&redact(&text, secrets)).to_owned()
}

pub async fn upload(client: &reqwest::Client, content: &str) -> Result<UploadedLog> {
    let http = |source| Error::Http { url: UPLOAD_URL.to_owned(), source };
    let body = serde_json::json!({ "content": content, "source": "_k0t1k" });
    let response = client.post(UPLOAD_URL).json(&body).send().await.map_err(http)?;
    // mclo.gs answers errors with a JSON body as well, so the status is not checked before parsing.
    let body = response.text().await.map_err(http)?;
    parse_response(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_and_error_responses_parse() {
        let ok = parse_response(r#"{"success":true,"id":"abc","url":"https://mclo.gs/abc","raw":"https://api.mclo.gs/1/raw/abc","size":3}"#)
            .unwrap();
        assert_eq!(ok, UploadedLog { id: "abc".into(), url: "https://mclo.gs/abc".into(), raw: "https://api.mclo.gs/1/raw/abc".into() });

        let error = parse_response(r#"{"success":false,"error":"Required field 'content' not found."}"#).unwrap_err();
        assert!(error.to_string().contains("Required field 'content' not found."));
        assert!(parse_response(r#"{"success":true}"#).is_err());
        assert!(parse_response("<html>").is_err());
    }

    #[test]
    fn access_tokens_and_secrets_are_masked() {
        let text = "java --username Bob --accessToken eyJhbGciOi.secret --uuid 1\nlast --accessToken\nsession token=hunter2hunter2 ok";
        let out = redact(text, &["hunter2hunter2", "short"]);
        assert_eq!(out, "java --username Bob --accessToken *** --uuid 1\nlast --accessToken\nsession token=*** ok");
        assert_eq!(redact("short word", &["short"]), "short word");
    }

    #[test]
    fn only_the_newest_lines_are_kept() {
        let many: Vec<String> = (0..MAX_LINES + 5).map(|n| n.to_string()).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let out = prepare_content(&refs, None, &[]);
        assert_eq!(out.lines().count(), MAX_LINES);
        assert_eq!(out.lines().next(), Some("5"));
        assert_eq!(out.lines().last(), Some(many.last().unwrap().as_str()));
    }

    #[test]
    fn byte_limit_cuts_at_a_line_start_and_keeps_the_crash_report() {
        let line = "x".repeat(1023);
        let lines: Vec<&str> = std::iter::repeat_n(line.as_str(), 10_300).collect();
        let out = prepare_content(&lines, Some("the crash"), &[]);
        assert!(out.len() <= MAX_BYTES);
        assert!(out.ends_with("the crash"));
        assert!(out.starts_with('x'));
        assert!(out.contains("===== Crash report ====="));
    }
}
