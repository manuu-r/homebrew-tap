//! Hook receiver: stores display-only events, never returns approval decisions.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub provider: String,
    pub session: String,
    pub project: String,
    pub message: String,
    pub kind: String,
    pub created_at: u64,
}
#[derive(Serialize, Deserialize)]
struct Event {
    at: u128,
    provider: String,
    session: String,
    key: String,
    clear_session: bool,
    request: Option<Request>,
}
#[derive(Clone, Default, Serialize)]
pub struct Attention {
    pub requests: Vec<Request>,
    pub errors: Vec<String>,
}
pub fn directory() -> Result<PathBuf, String> {
    Ok(crate::config::path()?.with_file_name("attention"))
}
fn clean(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}
fn identity(v: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    v["tool_input"].to_string().hash(&mut hash);
    format!(
        "{}:{:016x}",
        v["tool_name"].as_str().unwrap_or("notification"),
        hash.finish()
    )
}

fn event(provider: &str, v: &Value) -> Option<Event> {
    let session = v["session_id"].as_str()?.to_string();
    if session.is_empty() {
        return None;
    }
    let name = v["hook_event_name"].as_str()?;
    let tool = v["tool_name"].as_str().unwrap_or("");
    let question =
        tool.contains("request_user_input") || tool == "AskUserQuestion" || tool == "ExitPlanMode";
    let notification = v["notification_type"].as_str().unwrap_or("");
    let waiting = name == "PermissionRequest"
        || name == "Elicitation"
        || (name == "PreToolUse" && question)
        || (name == "Notification"
            && matches!(
                notification,
                "permission_prompt" | "idle_prompt" | "elicitation_dialog"
            ));
    let clear_session = matches!(
        name,
        "UserPromptSubmit" | "SessionStart" | "SessionEnd" | "Stop" | "Interrupt" | "StopFailure"
    );
    let clear_tool = matches!(
        name,
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" | "ElicitationResult"
    );
    if !waiting && !clear_session && !clear_tool {
        return None;
    }
    if clear_tool && tool.contains("request_user_input_async") {
        return None;
    }
    let key = identity(v);
    let project = v["cwd"].as_str().unwrap_or("");
    let message = v["tool_input"]["questions"]
        .as_array()
        .map(|qs| {
            qs.iter()
                .filter_map(|q| q["question"].as_str().or(q["title"].as_str()))
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .filter(|s| !s.is_empty())
        .or_else(|| v["message"].as_str().map(str::to_string))
        .or_else(|| v["tool_input"]["description"].as_str().map(str::to_string))
        .or_else(|| v["tool_input"]["command"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{tool} needs your response"));
    Some(Event {
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
        provider: provider.into(),
        session: session.clone(),
        key: key.clone(),
        clear_session,
        request: waiting.then(|| Request {
            id: key,
            provider: provider.into(),
            session,
            project: clean(project, 300),
            message: clean(&message, 1200),
            kind: if name == "PermissionRequest" || notification == "permission_prompt" {
                "Approval"
            } else {
                "Input"
            }
            .into(),
            created_at: crate::now_seconds(),
        }),
    })
}
pub fn receive(provider: &str) -> Result<(), String> {
    if !matches!(provider, "Codex" | "Claude") {
        return Err("unknown hook provider".into());
    }
    let mut input = String::new();
    std::io::stdin()
        .take(1_048_577)
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > 1_048_576 {
        return Err("hook payload exceeds 1 MiB".into());
    }
    let v: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    if let Some(event) = event(provider, &v) {
        save(&directory()?, &event)?;
    }
    Ok(())
}
fn save(dir: &Path, event: &Event) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| e.to_string())?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let id = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let path = dir.join(format!("{}-{id}.tmp", event.at));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec(event).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&path, path.with_extension("json")).map_err(|e| e.to_string())
}
pub fn collect(enabled: &crate::config::ProviderConfig) -> Attention {
    match directory() {
        Ok(dir) => read(&dir, enabled, crate::now_seconds()),
        Err(e) => Attention {
            errors: vec![e],
            ..Default::default()
        },
    }
}
fn read(dir: &Path, enabled: &crate::config::ProviderConfig, now: u64) -> Attention {
    let mut result = Attention::default();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return result,
        Err(e) => {
            result.errors.push(e.to_string());
            return result;
        }
    };
    let mut events = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(e.to_string());
                continue;
            }
        };
        if entry.path().extension().is_none_or(|s| s != "json") {
            continue;
        }
        match fs::read(entry.path())
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).map_err(|e| e.to_string()))
        {
            Ok(e) => {
                // Expired requests cannot stay in the badge forever after a killed agent.
                if now.saturating_sub((e.at / 1_000_000_000) as u64) > 86_400 {
                    let _ = fs::remove_file(entry.path());
                } else {
                    events.push(e);
                }
            }
            Err(e) => result.errors.push(e),
        }
    }
    events.sort_by_key(|e| e.at);
    let mut pending = BTreeMap::<(String, String, String), Request>::new();
    for e in events {
        if (e.provider == "Codex" && !enabled.codex) || (e.provider == "Claude" && !enabled.claude)
        {
            continue;
        }
        if e.clear_session {
            pending.retain(|(p, s, _), _| p != &e.provider || s != &e.session);
        } else {
            let key = (e.provider, e.session, e.key);
            if let Some(r) = e.request {
                pending.insert(key, r);
            } else {
                pending.remove(&key);
            }
        }
    }
    result.requests = pending.into_values().collect();
    result
        .requests
        .sort_by_key(|r| std::cmp::Reverse(r.created_at));
    result
}

/// Adds only Gauge's handlers; unrelated settings and hooks survive unchanged.
fn is_gauge_claude_hook(handler: &Value) -> bool {
    handler["command"].as_str().is_some_and(|command| {
        command.starts_with('\'') && command.ends_with("/gauge' --hook claude")
    })
}

pub fn hook_config(mut config: Value, provider: &str, executable: &Path) -> Result<Value, String> {
    let object = config
        .as_object_mut()
        .ok_or("hook configuration must be a JSON object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks must be an object")?;
    let quoted = format!("'{}'", executable.to_string_lossy().replace('\'', "'\\''"));
    let command = format!("{quoted} --hook {}", provider.to_ascii_lowercase());
    let mut names = vec![
        "PermissionRequest",
        "PreToolUse",
        "PostToolUse",
        "UserPromptSubmit",
        "SessionStart",
        "SessionEnd",
        "Stop",
    ];
    if provider == "Codex" {
        names.push("Interrupt");
    } else {
        names.push("PostToolUseFailure");
        // Older Claude releases reject these keys, even with empty arrays.
        // Migrate our previous installation without deleting other handlers.
        for event in ["PermissionDenied", "Elicitation", "ElicitationResult"] {
            if let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) {
                for group in groups.iter_mut() {
                    if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        handlers.retain(|h| !is_gauge_claude_hook(h));
                    }
                }
                groups.retain(|g| g["hooks"].as_array().is_none_or(|h| !h.is_empty()));
                if groups.is_empty() {
                    hooks.remove(event);
                }
            }
        }
    }
    for name in names {
        let groups = hooks
            .entry(name)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("{name} hooks must be an array"))?;
        // Exact ownership marker is the Gauge command suffix; preserve other handlers in groups.
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|h| {
                    !h["command"].as_str().is_some_and(|s| {
                        s.ends_with(&format!(" --hook {}", provider.to_ascii_lowercase()))
                    })
                });
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|h| !h.is_empty()));
        let mut group = json!({"hooks":[{"type":"command","command":command,"timeout":3}]});
        if name == "PreToolUse" {
            group["matcher"] = json!("AskUserQuestion|ExitPlanMode|.*request_user_input.*");
        }
        groups.push(group);
    }
    Ok(config)
}
/// Whether Gauge's hooks are already registered with either agent, so the menu
/// stops offering a setup step that has been done.
pub fn hooks_installed() -> bool {
    [("codex", "hooks.json"), ("claude", "settings.json")]
        .iter()
        .any(|(provider, file)| {
            let path = crate::activity::provider_root(if *provider == "codex" { "Codex" } else { "Claude" })
                .join(file);
            fs::read_to_string(path).is_ok_and(|body| body.contains(&format!(" --hook {provider}")))
        })
}

pub fn install_hooks() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Validate both files before modifying either one.
    let mut writes = Vec::new();
    for (provider, file) in [("Codex", "hooks.json"), ("Claude", "settings.json")] {
        let path = crate::activity::provider_root(provider).join(file);
        let old = match fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let value = match &old {
            Some(b) => serde_json::from_slice(b).map_err(|e| format!("{}: {e}", path.display()))?,
            None => json!({}),
        };
        let updated = hook_config(value, provider, &exe)?;
        writes.push((
            path,
            old,
            serde_json::to_vec_pretty(&updated).map_err(|e| e.to_string())?,
        ));
    }
    for (path, old, bytes) in writes {
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        if let Some(old) = old {
            private_write(&path.with_extension(format!("gauge-backup-{suffix}")), &old)?;
        }
        let tmp = path.with_extension(format!("gauge-{suffix}.tmp"));
        private_write(&tmp, &bytes)?;
        fs::rename(&tmp, &path).map_err(|e| format!("Could not update {}: {e}. Some hooks may already be installed; retry setup after resolving this error.", path.display()))?;
    }
    Ok(())
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_and_question_lifecycle_isolated_by_session() {
        let dir = std::env::temp_dir().join(format!("gauge-attention-test-{}", std::process::id()));
        let v = json!({"session_id":"one","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"npm test"},"cwd":"/project"});
        save(&dir, &event("Codex", &v).unwrap()).unwrap();
        let mut second = v.clone();
        second["session_id"] = json!("two");
        save(&dir, &event("Codex", &second).unwrap()).unwrap();
        let enabled = crate::config::ProviderConfig::default();
        assert_eq!(read(&dir, &enabled, crate::now_seconds()).requests.len(), 2);
        assert!(read(
            &dir,
            &crate::config::ProviderConfig {
                codex: false,
                claude: true,
                cursor: true,
            },
            crate::now_seconds()
        )
        .requests
        .is_empty());
        let mut done = v;
        done["hook_event_name"] = json!("PostToolUse");
        save(&dir, &event("Codex", &done).unwrap()).unwrap();
        let remaining = read(&dir, &enabled, crate::now_seconds());
        assert_eq!(remaining.requests.len(), 1);
        assert_eq!(remaining.requests[0].session, "two");
        second["hook_event_name"] = json!("Stop");
        save(&dir, &event("Codex", &second).unwrap()).unwrap();
        assert!(read(&dir, &enabled, crate::now_seconds())
            .requests
            .is_empty());
        let ask = json!({"session_id":"three","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which target?"}]}});
        let request = event("Claude", &ask).unwrap().request.unwrap();
        assert_eq!(request.message, "Which target?");
        assert_eq!(request.kind, "Input");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn normal_tools_do_not_raise_alerts_and_disabled_providers_are_hidden() {
        assert!(event(
            "Codex",
            &json!({"session_id":"s","hook_event_name":"PreToolUse","tool_name":"Bash"})
        )
        .is_none());
        assert!(event("Codex", &json!({"hook_event_name":"PermissionRequest"})).is_none());
    }
    #[test]
    fn claude_setup_migrates_unsupported_events_and_preserves_other_handlers() {
        let gauge = json!({"type":"command","command":"'/tmp/gauge' --hook claude"});
        let other = json!({"type":"command","command":"other-tool"});
        let old = json!({"theme":"dark","hooks":{
            "Elicitation":[{"hooks":[gauge.clone()]}],
            "ElicitationResult":[{"hooks":[gauge.clone()]}],
            "PermissionDenied":[{"hooks":[gauge,other.clone()]}]
        }});
        let updated = hook_config(old, "Claude", Path::new("/tmp/gauge")).unwrap();
        assert!(updated["hooks"].get("Elicitation").is_none());
        assert!(updated["hooks"].get("ElicitationResult").is_none());
        assert_eq!(
            updated["hooks"]["PermissionDenied"][0]["hooks"],
            json!([other])
        );
        assert_eq!(updated["theme"], "dark");
        let fresh = hook_config(json!({}), "Claude", Path::new("/tmp/gauge")).unwrap();
        for event in ["Elicitation", "ElicitationResult", "PermissionDenied"] {
            assert!(fresh["hooks"].get(event).is_none());
        }
        assert!(fresh["hooks"]["PermissionRequest"].is_array());
        assert_eq!(
            hook_config(fresh.clone(), "Claude", Path::new("/tmp/gauge")).unwrap(),
            fresh
        );
    }

    #[test]
    fn install_is_idempotent_and_preserves_user_hooks() {
        let original = json!({"theme":"dark","hooks":{"PermissionRequest":[{"hooks":[{"type":"command","command":"existing-tool"}]}]}});
        let first = hook_config(original, "Codex", Path::new("/tmp/Gauge's app/gauge")).unwrap();
        let second =
            hook_config(first.clone(), "Codex", Path::new("/tmp/Gauge's app/gauge")).unwrap();
        assert_eq!(first, second);
        assert_eq!(second["theme"], "dark");
        assert_eq!(
            second["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "existing-tool"
        );
        assert!(
            second["hooks"]["PermissionRequest"][1]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("'\\''")
        );
        assert!(hook_config(json!({"hooks":[]}), "Codex", Path::new("/tmp/gauge")).is_err());
    }
}
