//! Claude quota, read from the endpoint behind Claude Code's `/usage` screen.
//!
//! This reuses the OAuth token Claude Code stored at sign-in. Nothing is ever
//! written back, and the token is handed to curl over stdin so it stays out of
//! the process table.

use crate::Limit;
use serde_json::Value;
use std::{
    env, fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

pub fn fetch() -> Result<Vec<Limit>, String> {
    let token = access_token()?;
    parse(&request_usage(&token)?)
}

fn access_token() -> Result<String, String> {
    let raw = credentials()?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("unable to parse Claude credentials: {error}"))?;
    let oauth = parsed
        .get("claudeAiOauth")
        .ok_or("Claude credentials hold no OAuth session; sign in with `claude` first")?;

    // Only Claude Code refreshes this token, so name the fix rather than
    // letting the request come back as an opaque 401.
    let expired = oauth
        .get("expiresAt")
        .and_then(Value::as_u64)
        .is_some_and(|expiry| expiry <= now_millis());
    if expired {
        return Err("Claude OAuth token expired; start `claude` once to refresh it".to_string());
    }

    match oauth.get("accessToken").and_then(Value::as_str) {
        Some(token) if !token.is_empty() => Ok(token.to_string()),
        _ => Err("Claude credentials hold no access token".to_string()),
    }
}

/// The token lives in the macOS keychain, or in a plain file elsewhere.
fn credentials() -> Result<String, String> {
    let account = env::var("USER").unwrap_or_default();
    let keychain = Command::new("security")
        .args(["find-generic-password", "-a", &account, "-w", "-s"])
        .arg(KEYCHAIN_SERVICE)
        .output();

    if let Ok(output) = keychain {
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    let path = config_dir().join(".credentials.json");
    fs::read_to_string(&path).map_err(|error| {
        format!(
            "no keychain entry and {} is unreadable ({error}); sign in with `claude` first",
            path.display()
        )
    })
}

fn config_dir() -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").unwrap_or_default()).join(".claude"))
}

fn request_usage(token: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch curl: {error}"))?;

    // Curl config values are double-quoted, so escape what would end the quote.
    let config = format!(
        "url = \"{USAGE_URL}\"\n\
         header = \"Authorization: Bearer {}\"\n\
         header = \"anthropic-beta: {OAUTH_BETA}\"\n\
         user-agent = \"gauge/{}\"\n\
         max-time = 10\n\
         write-out = \"\\n%{{http_code}}\"\n",
        token.replace('\\', "\\\\").replace('"', "\\\""),
        env!("CARGO_PKG_VERSION"),
    );
    child
        .stdin
        .take()
        .ok_or("curl stdin unavailable")?
        .write_all(config.as_bytes())
        .map_err(|error| format!("failed writing curl config: {error}"))?;

    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed waiting for curl: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not reach the usage endpoint: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    split_status(output.stdout)
}

/// `write-out` appends `\n<http_code>` after the body; peel it back off.
fn split_status(mut body: Vec<u8>) -> Result<Vec<u8>, String> {
    let separator = body
        .iter()
        .rposition(|byte| *byte == b'\n')
        .ok_or("usage endpoint sent an empty response")?;
    let status = String::from_utf8_lossy(&body[separator + 1..])
        .trim()
        .parse::<u16>()
        .map_err(|_| "usage endpoint sent no HTTP status")?;
    body.truncate(separator);

    match status {
        200 => Ok(body),
        401 | 403 => Err("Claude OAuth token rejected; start `claude` once to refresh it".into()),
        _ => Err(format!(
            "usage endpoint returned HTTP {status}: {}",
            String::from_utf8_lossy(&body).trim()
        )),
    }
}

/// `limits` is the normalized view of every metered window.
pub fn parse(body: &[u8]) -> Result<Vec<Limit>, String> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|error| format!("unable to parse Claude usage: {error}"))?;

    let limits: Vec<Limit> = payload
        .get("limits")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|entry| {
            Some(Limit {
                label: label(entry.get("kind")?.as_str()?),
                used_percent: entry.get("percent")?.as_f64()?,
            })
        })
        .collect();

    match limits.is_empty() {
        true => Err("Claude reported no usage windows (subscription plans only)".to_string()),
        false => Ok(limits),
    }
}

fn label(kind: &str) -> String {
    match kind {
        "session" => "5-hour".to_string(),
        "weekly_all" => "Weekly".to_string(),
        "weekly_opus" => "Weekly (Opus)".to_string(),
        "weekly_sonnet" => "Weekly (Sonnet)".to_string(),
        other => other.replace('_', " "),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
