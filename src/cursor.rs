//! Cursor plan quota, from the account Cursor is signed into.
//!
//! The access token is read from Cursor's local state database and passed to
//! curl on stdin, the same way Claude's token is. Gauge never writes it back
//! and never stores a copy. Token history is not read from Cursor.

use crate::Limit;
use chrono::{DateTime, Local, Utc};
use serde_json::Value;
use std::{
    env,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const USAGE_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";

/// One Cursor request. Cached prompt tokens are omitted: Gauge counts new
/// input only, matching Codex and Claude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageEvent {
    pub key: String,
    pub at: DateTime<Local>,
    pub model: String,
    pub input: u64,
    pub output: u64,
}

pub fn fetch() -> Result<Vec<Limit>, String> {
    let token = access_token()?;
    parse_period(&request(&token, USAGE_URL, "{}")?)
}

fn access_token() -> Result<String, String> {
    let path = state_db();
    let output = Command::new("/usr/bin/sqlite3")
        .arg(sqlite_uri(&path))
        .arg("SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken' LIMIT 1;")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("could not read Cursor sign-in: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not read Cursor sign-in ({}); open Cursor and sign in first",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() || token.split('.').count() != 3 {
        return Err("Cursor is not signed in; open Cursor and sign in first".into());
    }
    Ok(token)
}

fn state_db() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default())
        .join("Library")
        .join("Application Support")
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

fn sqlite_uri(path: &Path) -> String {
    let encoded = path.to_string_lossy().replace(' ', "%20");
    format!("file:{encoded}?mode=ro")
}

fn request(token: &str, url: &str, body: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch curl: {error}"))?;

    let config = format!(
        "url = \"{url}\"\n\
         request = \"POST\"\n\
         header = \"Authorization: Bearer {}\"\n\
         header = \"Content-Type: application/json\"\n\
         header = \"Connect-Protocol-Version: 1\"\n\
         data = \"{body}\"\n\
         user-agent = \"gauge/{}\"\n\
         max-time = 20\n\
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
            "could not reach Cursor: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    split_status(output.stdout)
}

fn split_status(mut body: Vec<u8>) -> Result<Vec<u8>, String> {
    let separator = body
        .iter()
        .rposition(|byte| *byte == b'\n')
        .ok_or("Cursor sent an empty response")?;
    let status = String::from_utf8_lossy(&body[separator + 1..])
        .trim()
        .parse::<u16>()
        .map_err(|_| "Cursor sent no HTTP status")?;
    body.truncate(separator);
    match status {
        200 => Ok(body),
        401 | 403 => Err("Cursor session was rejected; open Cursor and sign in again".into()),
        _ => Err(format!(
            "Cursor returned HTTP {status}: {}",
            String::from_utf8_lossy(&body).trim()
        )),
    }
}

pub fn parse_period(body: &[u8]) -> Result<Vec<Limit>, String> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|error| format!("unable to parse Cursor usage: {error}"))?;
    let plan = payload
        .get("planUsage")
        .ok_or("Cursor reported no plan usage")?;
    let limit = json_f64(plan.get("limit").unwrap_or(&Value::Null));
    if limit <= 0.0 {
        return Err("Cursor reported no included usage limit".into());
    }
    let spent = json_f64(plan.get("includedSpend").unwrap_or(&Value::Null));
    Ok(vec![Limit {
        label: "Included".into(),
        used_percent: (spent / limit * 100.0).clamp(0.0, 100.0),
        resets_at: payload.get("billingCycleEnd").and_then(epoch_seconds),
    }])
}

pub fn parse_events(body: &[u8]) -> Result<(usize, Vec<UsageEvent>), String> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|error| format!("unable to parse Cursor usage history: {error}"))?;
    let total = payload
        .get("totalUsageEventsCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let events = payload
        .get("usageEventsDisplay")
        .and_then(Value::as_array)
        .map(|events| events.iter().filter_map(parse_event).collect())
        .unwrap_or_default();
    Ok((total, events))
}

fn parse_event(event: &Value) -> Option<UsageEvent> {
    let at = epoch_seconds(event.get("timestamp")?)?;
    let at = DateTime::<Utc>::from_timestamp(i64::try_from(at).ok()?, 0)?.with_timezone(&Local);
    let usage = event.get("tokenUsage")?;
    let input = json_u64(usage.get("inputTokens").unwrap_or(&Value::Null));
    let output = json_u64(usage.get("outputTokens").unwrap_or(&Value::Null));
    if input == 0 && output == 0 {
        return None;
    }
    let model = event
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .unwrap_or("Unknown");
    let conversation = event
        .get("conversationId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let timestamp = event.get("timestamp").and_then(Value::as_str).unwrap_or("");
    Some(UsageEvent {
        key: format!("Cursor:{conversation}:{timestamp}:{model}"),
        at,
        model: model.to_string(),
        input,
        output,
    })
}

fn json_f64(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or(0.0)
}

fn json_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or(0)
}

fn epoch_seconds(value: &Value) -> Option<u64> {
    let raw = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))?;
    Some(if raw >= 100_000_000_000 {
        raw / 1_000
    } else {
        raw
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_events, parse_period};

    #[test]
    fn included_spend_is_the_plan_meter() {
        let body = br#"{
            "billingCycleStart": "1789258822000",
            "billingCycleEnd": "1791850822000",
            "planUsage": { "includedSpend": 394, "limit": 2000, "totalPercentUsed": 0.83 }
        }"#;
        let limits = parse_period(body).unwrap();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].label, "Included");
        assert!((limits[0].used_percent - 19.7).abs() < 0.01);
        assert_eq!(limits[0].resets_at, Some(1_791_850_822));
    }

    #[test]
    fn usage_events_ignore_cached_reads_and_empty_rows() {
        let body = br#"{
            "totalUsageEventsCount": 2,
            "usageEventsDisplay": [
                {
                    "timestamp": "1790054441605",
                    "model": "grok-4.7-high-fast",
                    "conversationId": "conv",
                    "tokenUsage": { "inputTokens": 10, "outputTokens": 4, "cacheReadTokens": 900 }
                },
                {
                    "timestamp": "1790054441605",
                    "model": "empty",
                    "tokenUsage": { "inputTokens": 0, "outputTokens": 0, "cacheReadTokens": 50 }
                }
            ]
        }"#;
        let (total, events) = parse_events(body).unwrap();
        assert_eq!(total, 2);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].input, 10);
        assert_eq!(events[0].output, 4);
        assert_eq!(events[0].model, "grok-4.7-high-fast");
    }
}
