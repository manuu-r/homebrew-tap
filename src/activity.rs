//! Local token accounting. Transcript formats are best-effort, not billing APIs.
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Tokens {
    /// Includes cache reads and writes, for both providers.
    pub input: u64,
    pub output: u64,
    /// Subset of input; never add again when computing total.
    pub cache_read: u64,
    pub cache_write: u64,
    /// Subset of output.
    pub reasoning: u64,
}
impl Tokens {
    pub fn total(self) -> u64 {
        self.input.saturating_add(self.output)
    }
    fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
    }
    fn delta(self, old: Self) -> Self {
        Self {
            input: self.input.saturating_sub(old.input),
            output: self.output.saturating_sub(old.output),
            cache_read: self.cache_read.saturating_sub(old.cache_read),
            cache_write: self.cache_write.saturating_sub(old.cache_write),
            reasoning: self.reasoning.saturating_sub(old.reasoning),
        }
    }
    fn max(self, other: Self) -> Self {
        Self {
            input: self.input.max(other.input),
            output: self.output.max(other.output),
            cache_read: self.cache_read.max(other.cache_read),
            cache_write: self.cache_write.max(other.cache_write),
            reasoning: self.reasoning.max(other.reasoning),
        }
    }
}
#[derive(Clone)]
struct Record {
    key: String,
    at: DateTime<Local>,
    provider: String,
    model: String,
    tokens: Tokens,
}
#[derive(Clone, Default, Serialize)]
pub struct Period {
    pub tokens: Tokens,
    pub requests: u64,
}
impl Period {
    fn add(&mut self, tokens: Tokens) {
        self.tokens.add(tokens);
        self.requests += 1;
    }
}
#[derive(Clone, Default, Serialize)]
pub struct ProviderStats {
    pub provider: String,
    pub today: Period,
    pub week: Period,
    pub month: Period,
    pub all_time: Period,
}
#[derive(Clone, Serialize)]
pub struct DailyStats {
    pub date: String,
    pub provider: String,
    pub model: String,
    pub usage: Period,
}
#[derive(Clone, Default, Serialize)]
pub struct TokenStats {
    pub providers: Vec<ProviderStats>,
    pub daily: Vec<DailyStats>,
    pub errors: Vec<String>,
    pub files: usize,
}
#[derive(Default)]
struct ParseState {
    records: HashMap<String, Record>,
    /// Codex's older `token_count` events. Newer sessions also write per-response
    /// `token_usage_record`s, and the two disagree by a few tokens at a time, so a
    /// file's legacy events are used only when it has no records at all.
    legacy: HashMap<String, Record>,
    previous: Tokens,
    model: String,
    bad: usize,
    offset: u64,
}
struct CachedFile {
    modified: Option<SystemTime>,
    inode: u64,
    len: u64,
    state: ParseState,
}
#[derive(Default)]
pub struct UsageReader {
    cache: HashMap<PathBuf, CachedFile>,
}

pub fn provider_root(provider: &str) -> PathBuf {
    let (var, dir) = if provider == "Codex" {
        ("CODEX_HOME", ".codex")
    } else {
        ("CLAUDE_CONFIG_DIR", ".claude")
    };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(dir))
}
fn files(root: &Path, output: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            errors.push(format!("{}: {e}", root.display()));
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => match entry.file_type() {
                Ok(t) if t.is_dir() => files(&entry.path(), output, errors),
                Ok(t) if t.is_file() && entry.path().extension().is_some_and(|x| x == "jsonl") => {
                    output.push(entry.path())
                }
                Err(e) => errors.push(e.to_string()),
                _ => {}
            },
            Err(e) => errors.push(e.to_string()),
        }
    }
}
impl UsageReader {
    pub fn collect(&mut self, enabled: &crate::config::ProviderConfig) -> TokenStats {
        let mut roots = Vec::new();
        if enabled.codex {
            roots.push(("Codex", provider_root("Codex").join("sessions")));
            roots.push(("Codex", provider_root("Codex").join("archived_sessions")));
        }
        if enabled.claude {
            roots.push(("Claude", provider_root("Claude").join("projects")));
        }
        self.collect_roots(&roots, Local::now())
    }
    pub fn collect_roots(&mut self, roots: &[(&str, PathBuf)], now: DateTime<Local>) -> TokenStats {
        let mut stats = TokenStats::default();
        let mut records = HashMap::<String, Record>::new();
        let mut live = std::collections::HashSet::new();
        for (provider, root) in roots {
            let mut paths = Vec::new();
            files(root, &mut paths, &mut stats.errors);
            for path in paths {
                live.insert(path.clone());
                stats.files += 1;
                let metadata = match fs::metadata(&path) {
                    Ok(m) => m,
                    Err(e) => {
                        stats.errors.push(e.to_string());
                        continue;
                    }
                };
                let changed = self.cache.get(&path).is_none_or(|c| {
                    c.modified != metadata.modified().ok() || c.len != metadata.len()
                });
                if changed {
                    use std::os::unix::fs::MetadataExt;
                    let old = self.cache.remove(&path);
                    let mut state = match old {
                        Some(c) if c.inode == metadata.ino() && metadata.len() > c.len => c.state,
                        _ => ParseState::default(),
                    };
                    match fs::File::open(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|mut file| {
                            file.seek(SeekFrom::Start(state.offset))
                                .map_err(|e| e.to_string())?;
                            parse_append(BufReader::new(file), provider, &mut state)
                        }) {
                        Ok(()) => {
                            self.cache.insert(
                                path.clone(),
                                CachedFile {
                                    modified: metadata.modified().ok(),
                                    inode: metadata.ino(),
                                    len: metadata.len(),
                                    state,
                                },
                            );
                        }
                        Err(e) => stats.errors.push(format!("{}: {e}", path.display())),
                    }
                }
                if let Some(cached) = self.cache.get(&path) {
                    if cached.state.bad > 0 {
                        stats.errors.push(format!(
                            "{}: {} unreadable records",
                            path.display(),
                            cached.state.bad
                        ));
                    }
                    let file_records = if cached.state.records.is_empty() {
                        &cached.state.legacy
                    } else {
                        &cached.state.records
                    };
                    for record in file_records.values() {
                        records
                            .entry(record.key.clone())
                            .and_modify(|old| old.tokens = old.tokens.max(record.tokens))
                            .or_insert_with(|| record.clone());
                    }
                }
            }
        }
        self.cache.retain(|path, _| live.contains(path));
        let today = now.date_naive();
        let week = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let month = today.with_day(1).unwrap();
        let mut providers = BTreeMap::<String, ProviderStats>::new();
        for (name, _) in roots {
            providers
                .entry(name.to_string())
                .or_insert_with(|| ProviderStats {
                    provider: name.to_string(),
                    ..Default::default()
                });
        }
        let mut daily = BTreeMap::<(NaiveDate, String, String), Period>::new();
        for record in records.values().filter(|r| r.at <= now) {
            let p = providers.get_mut(&record.provider).unwrap();
            let day = record.at.date_naive();
            p.all_time.add(record.tokens);
            if day == today {
                p.today.add(record.tokens);
            }
            if day >= week {
                p.week.add(record.tokens);
            }
            if day >= month {
                p.month.add(record.tokens);
            }
            daily
                .entry((day, record.provider.clone(), record.model.clone()))
                .or_default()
                .add(record.tokens);
        }
        stats.providers = providers.into_values().collect();
        stats.daily = daily
            .into_iter()
            .rev()
            .map(|((date, provider, model), usage)| DailyStats {
                date: date.to_string(),
                provider,
                model,
                usage,
            })
            .collect();
        stats
    }
}
fn num(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
fn tokens(v: &Value, claude: bool) -> Tokens {
    let cache_read = num(
        v,
        if claude {
            "cache_read_input_tokens"
        } else {
            "cached_input_tokens"
        },
    );
    let cache_write = num(
        v,
        if claude {
            "cache_creation_input_tokens"
        } else {
            "cache_write_input_tokens"
        },
    );
    Tokens {
        input: num(v, "input_tokens").saturating_add(if claude {
            cache_read.saturating_add(cache_write)
        } else {
            0
        }),
        output: num(v, "output_tokens"),
        cache_read,
        cache_write,
        reasoning: if claude {
            num(&v["output_tokens_details"], "thinking_tokens")
        } else {
            num(v, "reasoning_output_tokens")
        },
    }
}
fn parse_append(
    mut reader: impl BufRead,
    provider: &str,
    state: &mut ParseState,
) -> Result<(), String> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 || !line.ends_with(b"\n") {
            break;
        }
        state.offset += count as u64;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let v: Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(_) => {
                state.bad += 1;
                continue;
            }
        };
        if v["type"] == "turn_context" {
            if let Some(m) = v["payload"]["model"].as_str() {
                state.model = m.into();
            }
        }
        let Some(at) = v["timestamp"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Local))
        else {
            continue;
        };
        let (key, usage, record_model) = if provider == "Claude"
            && v["type"] == "assistant"
            && v["message"]["usage"].is_object()
        {
            let Some(id) = v["message"]["id"].as_str() else {
                continue;
            };
            (
                format!("Claude:{id}:{}", v["requestId"].as_str().unwrap_or("")),
                tokens(&v["message"]["usage"], true),
                v["message"]["model"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string(),
            )
        } else if provider == "Codex" && v["type"] == "token_usage_record" {
            let payload = &v["payload"];
            let Some(id) = payload["response_id"].as_str() else {
                continue;
            };
            if !payload["usage"].is_object() {
                continue;
            }
            (
                format!("Codex:{id}"),
                tokens(&payload["usage"], false),
                if state.model.is_empty() {
                    "Unknown".into()
                } else {
                    state.model.clone()
                },
            )
        } else if provider == "Codex"
            && v["type"] == "event_msg"
            && v["payload"]["type"] == "token_count"
        {
            let total = &v["payload"]["info"]["total_token_usage"];
            if !total.is_object() {
                continue;
            }
            let current = tokens(total, false);
            let last = &v["payload"]["info"]["last_token_usage"];
            // Prefer the per-response figure Codex already reports. Deriving it
            // from the running total is only for logs that predate it, and a
            // total that goes backwards there means a new counter.
            let usage = if last.is_object() {
                tokens(last, false)
            } else if current.input < state.previous.input {
                current
            } else {
                current.delta(state.previous)
            };
            state.previous = current;
            if usage.total() == 0 {
                continue;
            }
            // Quota-only events repeat the same running total; keying on it
            // counts each response once, even when a forked session copies it.
            let key = format!(
                "Codex:{}:{}:{}",
                current.input, current.output, current.cache_read
            );
            let model = if state.model.is_empty() {
                "Unknown".into()
            } else {
                state.model.clone()
            };
            state.legacy.entry(key.clone()).or_insert(Record {
                key,
                at,
                provider: provider.into(),
                model,
                tokens: usage,
            });
            continue;
        } else {
            continue;
        };
        state
            .records
            .entry(key.clone())
            .and_modify(|r| r.tokens = r.tokens.max(usage))
            .or_insert(Record {
                key,
                at,
                provider: provider.into(),
                model: record_model,
                tokens: usage,
            });
    }
    Ok(())
}
pub fn compact(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1e9)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1e3)
    } else {
        value.to_string()
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Monitoring {
    pub tokens: TokenStats,
    pub attention: crate::attention::Attention,
    pub ready: bool,
}
impl Monitoring {
    pub fn retain_enabled(&mut self, enabled: &crate::config::ProviderConfig) {
        let show = |provider: &str| match provider {
            "Codex" => enabled.codex,
            "Claude" => enabled.claude,
            _ => false,
        };
        self.tokens.providers.retain(|p| show(&p.provider));
        self.tokens.daily.retain(|p| show(&p.provider));
        self.attention.requests.retain(|r| show(&r.provider));
    }
    pub fn title(&self, quota: &str) -> String {
        if self.attention.requests.is_empty() {
            quota.into()
        } else {
            format!(
                "● {} need attention · {quota}",
                self.attention.requests.len()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn parse_values(provider: &str, values: Vec<Value>) -> Vec<Record> {
        let mut state = ParseState::default();
        let input = values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        parse_append(std::io::Cursor::new(input), provider, &mut state).unwrap();
        if state.records.is_empty() {
            state.legacy.into_values().collect()
        } else {
            state.records.into_values().collect()
        }
    }
    fn codex(at: &str, input: u64, output: u64) -> Value {
        json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":input/2}}}})
    }
    #[test]
    fn cumulative_snapshots_deduplicate_and_reset() {
        let rows = parse_values(
            "Codex",
            vec![
                codex("2026-09-14T10:00:00Z", 100, 20),
                codex("2026-09-14T10:01:00Z", 100, 20),
                codex("2026-09-15T10:00:00Z", 150, 30),
                codex("2026-09-15T10:01:00Z", 10, 2),
            ],
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|r| r.tokens.total()).sum::<u64>(), 192);
    }
    #[test]
    fn a_running_total_that_disagrees_with_records_is_never_recounted() {
        let record = |id: &str, turn: u64, thread: u64| json!({"type":"token_usage_record","timestamp":"2026-09-15T10:00:00Z","payload":{"response_id":id,"usage":{"input_tokens":turn,"output_tokens":0},"thread_token_usage":{"input_tokens":thread,"output_tokens":5}}});
        // The legacy mirror reports one fewer output token than the records,
        // which used to look like a counter reset and re-add the whole session.
        let rows = parse_values(
            "Codex",
            vec![
                record("r1", 1_000_000, 1_000_000),
                codex("2026-09-15T10:00:01Z", 1_000_000, 4),
                record("r2", 1_000_000, 2_000_000),
                codex("2026-09-15T10:00:02Z", 2_000_000, 4),
            ],
        );
        assert_eq!(
            rows.iter().map(|r| r.tokens.total()).sum::<u64>(),
            2_000_000
        );
    }

    #[test]
    fn modern_codex_records_do_not_double_count_legacy_mirror() {
        let modern = json!({"type":"token_usage_record","timestamp":"2026-09-15T10:00:00Z","payload":{"response_id":"response1","usage":{"input_tokens":100,"output_tokens":20},"thread_token_usage":{"input_tokens":100,"output_tokens":20}}});
        let rows = parse_values(
            "Codex",
            vec![
                modern.clone(),
                codex("2026-09-15T10:00:01Z", 100, 20),
                modern,
                codex("2026-09-15T10:00:02Z", 100, 20),
            ],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tokens.total(), 120);
    }
    #[test]
    fn claude_chunks_merge_and_cache_is_counted_once() {
        let base = json!({"type":"assistant","timestamp":"2026-09-15T10:00:00Z","requestId":"request","message":{"id":"msg1","model":"claude-test","usage":{"input_tokens":2,"output_tokens":3,"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}});
        let mut end = base.clone();
        end["message"]["usage"]["output_tokens"] = json!(10);
        let rows = parse_values("Claude", vec![base, end]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tokens.total(), 132);
        assert_eq!(rows[0].tokens.cache_read, 100);
    }
    #[test]
    fn appended_records_wait_for_newline_and_rewrites_reset_cache() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("gauge-append-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let first = codex("2026-09-14T12:00:00Z", 100, 20).to_string() + "\n";
        let next = codex("2026-09-15T12:00:00Z", 200, 40).to_string() + "\n";
        fs::write(&path, &first).unwrap();
        let mut reader = UsageReader::default();
        let roots = [("Codex", dir.clone())];
        let now = DateTime::parse_from_rfc3339("2026-09-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(
            reader.collect_roots(&roots, now).providers[0]
                .all_time
                .tokens
                .total(),
            120
        );
        let half = next.len() / 2;
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&next.as_bytes()[..half])
            .unwrap();
        let partial = reader.collect_roots(&roots, now);
        assert!(partial.errors.is_empty());
        assert_eq!(partial.providers[0].all_time.tokens.total(), 120);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&next.as_bytes()[half..])
            .unwrap();
        assert_eq!(
            reader.collect_roots(&roots, now).providers[0]
                .all_time
                .tokens
                .total(),
            240
        );
        fs::write(&path, &first).unwrap();
        assert_eq!(
            reader.collect_roots(&roots, now).providers[0]
                .all_time
                .tokens
                .total(),
            120
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn calendar_periods_duplicates_cache_and_disabled_providers() {
        let dir = std::env::temp_dir().join(format!("gauge-stats-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let rows = [
            codex("2026-08-31T12:00:00Z", 100, 0),
            codex("2026-09-01T12:00:00Z", 200, 0),
            codex("2026-09-14T12:00:00Z", 300, 0),
            codex("2026-09-15T12:00:00Z", 400, 0),
        ];
        let text = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(dir.join("a.jsonl"), &text).unwrap();
        fs::write(dir.join("duplicate.jsonl"), &text).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-15T23:59:00+05:30")
            .unwrap()
            .with_timezone(&Local);
        let mut reader = UsageReader::default();
        let roots = [("Codex", dir.clone())];
        let stats = reader.collect_roots(&roots, now);
        let p = &stats.providers[0];
        assert_eq!(p.all_time.tokens.total(), 400);
        assert_eq!(p.month.tokens.total(), 300);
        assert_eq!(p.week.tokens.total(), 200);
        assert_eq!(p.today.tokens.total(), 100);
        assert_eq!(
            reader.collect_roots(&roots, now).providers[0]
                .all_time
                .tokens
                .total(),
            400
        );
        assert!(reader.collect_roots(&[], now).providers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
