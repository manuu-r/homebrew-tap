//! Reads how much of each agent's quota is left.
//!
//! Each provider talks to its own vendor tooling and hands back the windows it
//! meters; everything downstream is just formatting.

pub mod claude;
pub mod codex;

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use std::time::{SystemTime, UNIX_EPOCH};

/// One rate-limit window, such as Codex's weekly quota.
pub struct Limit {
    pub label: String,
    pub used_percent: f64,
    /// Unix timestamp, in seconds, for the end of this metering window.
    pub resets_at: Option<u64>,
}

impl Limit {
    pub fn remaining_percent(&self) -> u64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0).round() as u64
    }
}

/// One agent's quota.
pub struct Usage {
    pub name: &'static str,
    pub limits: Vec<Limit>,
}

impl Usage {
    /// Percentage left in the tightest window, which is the only one that can
    /// actually stop you. Separate model buckets such as Codex Spark do not
    /// replace the provider's main quota in compact summaries.
    pub fn remaining_percent(&self) -> Option<u64> {
        remaining(
            self.limits
                .iter()
                .filter(|limit| self.name != "Codex" || !is_spark(limit)),
        )
        .or_else(|| remaining(self.limits.iter()))
    }
}

fn remaining<'a>(limits: impl Iterator<Item = &'a Limit>) -> Option<u64> {
    limits.map(|limit| limit.remaining_percent()).min()
}

fn is_spark(limit: &Limit) -> bool {
    limit.label.starts_with("Spark ")
}

fn is_hourly(limit: &Limit) -> bool {
    limit.label.to_ascii_lowercase().contains("hour")
}

/// Compact menu-bar text: Claude's short window and the regular Codex quota.
pub fn tray_summary(usages: &[Usage]) -> String {
    let parts: Vec<_> = usages
        .iter()
        .filter_map(|usage| {
            let remaining = match usage.name {
                "Claude" => remaining(usage.limits.iter().filter(|limit| is_hourly(limit))),
                _ => usage.remaining_percent(),
            }?;
            Some(format!("{} {remaining}%", usage.name))
        })
        .collect();

    match parts.is_empty() {
        true => "Agent quota unavailable".to_string(),
        false => parts.join(", "),
    }
}

/// One provider's short- and long-window rows for the expanded tray menu.
#[derive(Debug, PartialEq, Eq)]
pub struct QuotaGroup {
    pub provider: &'static str,
    pub hourly_rows: Vec<String>,
    pub weekly_rows: Vec<String>,
}

/// Provider groups and compact per-window rows for the expanded tray menu.
///
/// Native tray menus do not provide a grid widget, so the strings use fixed
/// columns: window, a short progress indicator, percentage, then reset time.
pub fn quota_groups(usages: &[Usage]) -> Vec<QuotaGroup> {
    let mut groups = Vec::new();

    for usage in usages {
        if usage.name == "Codex" {
            push_quota_group(
                &mut groups,
                "Codex",
                usage.limits.iter().filter(|limit| !is_spark(limit)),
            );
            push_quota_group(
                &mut groups,
                "Codex Spark",
                usage.limits.iter().filter(|limit| is_spark(limit)),
            );
        } else {
            push_quota_group(&mut groups, usage.name, usage.limits.iter());
        }
    }

    groups
}

fn push_quota_group<'a>(
    groups: &mut Vec<QuotaGroup>,
    provider: &'static str,
    limits: impl Iterator<Item = &'a Limit>,
) {
    let mut hourly_rows = Vec::new();
    let mut weekly_rows = Vec::new();

    for limit in limits {
        let label = if is_hourly(limit) { "Hourly" } else { "Weekly" };
        let remaining = limit.remaining_percent();
        let percentage_padding = " ".repeat(3 - remaining.to_string().len());
        let row = format!(
            "  {label:<6}  {}  {remaining}%{percentage_padding}  ({})",
            quota_bar(remaining),
            reset_time(limit),
        );
        if is_hourly(limit) {
            hourly_rows.push(row);
        } else {
            weekly_rows.push(row);
        }
    }

    if !hourly_rows.is_empty() || !weekly_rows.is_empty() {
        groups.push(QuotaGroup {
            provider,
            hourly_rows,
            weekly_rows,
        });
    }
}

fn reset_time(limit: &Limit) -> String {
    let Some(resets_at) = limit.resets_at else {
        return "—".to_string();
    };
    let Some(reset) = timestamp_local(resets_at) else {
        return "—".to_string();
    };

    match is_hourly(limit) {
        true => format!("resets {}", reset.format("%H:%M")),
        false => format!(
            "resets {}{} {}",
            reset.day(),
            ordinal_suffix(reset.day()),
            reset.format("%H:%M"),
        ),
    }
}

fn ordinal_suffix(day: u32) -> &'static str {
    match day % 100 {
        11..=13 => "th",
        _ => match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    }
}

fn timestamp_local(timestamp: u64) -> Option<DateTime<Local>> {
    let timestamp = i64::try_from(timestamp).ok()?;
    Utc.timestamp_opt(timestamp, 0).single().map(DateTime::from)
}

fn quota_bar(percent: u64) -> String {
    const SEGMENTS: usize = 6;
    let filled = ((percent.min(100) * SEGMENTS as u64 + 50) / 100) as usize;
    format!("{}{}", "▰".repeat(filled), "▱".repeat(SEGMENTS - filled))
}

/// Query every provider, keeping failures alongside successes so one missing
/// CLI never hides the other's numbers.
pub fn fetch_all() -> (Vec<Usage>, Vec<String>) {
    let mut usages = Vec::new();
    let mut errors = Vec::new();

    for (name, result) in [("Codex", codex::fetch()), ("Claude", claude::fetch())] {
        match result {
            Ok(limits) => usages.push(Usage { name, limits }),
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }

    (usages, errors)
}

pub fn summary(usages: &[Usage]) -> String {
    let parts: Vec<String> = usages
        .iter()
        .filter_map(|usage| Some(format!("{} {}%", usage.name, usage.remaining_percent()?)))
        .collect();

    match parts.is_empty() {
        true => "Agent quota unavailable".to_string(),
        false => parts.join(", "),
    }
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::ordinal_suffix;

    #[test]
    fn formats_calendar_ordinals() {
        assert_eq!(ordinal_suffix(1), "st");
        assert_eq!(ordinal_suffix(2), "nd");
        assert_eq!(ordinal_suffix(3), "rd");
        assert_eq!(ordinal_suffix(11), "th");
        assert_eq!(ordinal_suffix(21), "st");
        assert_eq!(ordinal_suffix(27), "th");
    }
}
