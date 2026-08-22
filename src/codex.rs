//! Codex quota, read over the `codex app-server` JSON-RPC protocol.

use crate::Limit;
use serde_json::Value;
use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{ChildStdin, Command, Stdio},
    sync::mpsc::Receiver,
    thread,
    time::{Duration, Instant},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// The rate-limit read goes out to the backend, so allow for a round trip.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

pub fn fetch() -> Result<Vec<Limit>, String> {
    let executable = find_executable().ok_or("codex CLI not found; install and sign in first")?;

    let mut child = Command::new(executable)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to launch codex: {error}"))?;

    let stdout = child.stdout.take().ok_or("codex stdout unavailable")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let result = child
        .stdin
        .take()
        .ok_or_else(|| "codex stdin unavailable".to_string())
        .and_then(|stdin| request_limits(stdin, &receiver));

    let _ = child.kill();
    let _ = child.wait();
    result
}

fn request_limits(
    mut stdin: ChildStdin,
    receiver: &Receiver<String>,
) -> Result<Vec<Limit>, String> {
    // The app-server serves nothing until `initialize` is answered and
    // acknowledged with an `initialized` notification.
    send(
        &mut stdin,
        &format!(
            r#"{{"id":1,"method":"initialize","params":{{"clientInfo":{{"name":"gauge","version":"{}"}}}}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )?;
    await_response(receiver, 1, HANDSHAKE_TIMEOUT)?;

    send(&mut stdin, r#"{"method":"initialized"}"#)?;
    send(
        &mut stdin,
        r#"{"id":2,"method":"account/rateLimits/read","params":{}}"#,
    )?;

    // Hold stdin open: closing it shuts the app-server down mid-answer.
    let response = await_response(receiver, 2, READ_TIMEOUT)?;
    parse(&response)
}

fn send(stdin: &mut ChildStdin, payload: &str) -> Result<(), String> {
    writeln!(stdin, "{payload}").map_err(|error| format!("failed writing to codex: {error}"))
}

fn await_response(
    receiver: &Receiver<String>,
    id: u64,
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("codex did not answer in time")?;
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|_| "codex did not answer in time")?;

        match serde_json::from_str::<Value>(&line) {
            Ok(value) if value.get("id").and_then(Value::as_u64) == Some(id) => return Ok(value),
            _ => continue,
        }
    }
}

pub fn parse(response: &Value) -> Result<Vec<Limit>, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("rate-limit request failed: {error}"));
    }
    let result = response.get("result").ok_or("codex sent no result")?;

    // Accounts metered across several buckets expose them by limit id; the
    // flat `rateLimits` view is the single-bucket equivalent. Spark is a
    // separate model bucket and must not replace the regular Codex limits.
    let buckets = result.get("rateLimitsByLimitId").and_then(Value::as_object);
    let regular = buckets
        .and_then(|buckets| buckets.get("codex"))
        .or_else(|| result.get("rateLimits"));
    let mut limits = regular
        .map(|bucket| windows(bucket, None))
        .unwrap_or_default();

    if let Some(buckets) = buckets {
        for bucket in buckets.values().filter(|bucket| spark_bucket(bucket)) {
            limits.extend(windows(bucket, Some("Spark")));
        }
    }

    match limits.is_empty() {
        true => Err("codex reported no usage windows".to_string()),
        false => Ok(limits),
    }
}

fn spark_bucket(bucket: &Value) -> bool {
    bucket
        .get("limitName")
        .and_then(Value::as_str)
        .is_some_and(|name| name.to_ascii_lowercase().contains("spark"))
}

fn windows(bucket: &Value, prefix: Option<&str>) -> Vec<Limit> {
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| window(bucket.get(key)?, prefix))
        .collect()
}

/// Only `usedPercent` is guaranteed; the window length may be absent or null.
fn window(window: &Value, prefix: Option<&str>) -> Option<Limit> {
    let label = label(window.get("windowDurationMins").and_then(Value::as_u64));
    Some(Limit {
        label: prefix.map_or(label.clone(), |prefix| format!("{prefix} {label}")),
        used_percent: window.get("usedPercent")?.as_f64()?,
        resets_at: reset_timestamp(window),
    })
}

fn reset_timestamp(window: &Value) -> Option<u64> {
    window.get("resetsAt").and_then(Value::as_u64)
}

fn label(minutes: Option<u64>) -> String {
    match minutes {
        Some(300) => "5-hour".to_string(),
        Some(10_080) => "Weekly".to_string(),
        Some(minutes) => format!("{}-hour", minutes / 60),
        None => "Usage window".to_string(),
    }
}

fn find_executable() -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|directory| directory.join("codex"))
        .find(|candidate| candidate.is_file())
}
