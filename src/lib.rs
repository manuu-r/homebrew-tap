//! Reads how much of each agent's quota is left.
//!
//! Each provider talks to its own vendor tooling and hands back the windows it
//! meters; everything downstream is just formatting.

pub mod activity;
pub mod attention;
pub mod calendar;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod config;
pub mod dashboard;
pub mod devices;
pub mod provisioning;

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use std::time::{SystemTime, UNIX_EPOCH};

/// One rate-limit window, such as Codex's weekly quota.
#[derive(Clone)]
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
#[derive(Clone)]
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
/// A window with nothing left is omitted. An empty title means every reported
/// window is exhausted; the unavailable line is only for when nothing was read.
pub fn tray_summary(usages: &[Usage]) -> String {
    let readings: Vec<_> = usages
        .iter()
        .filter_map(|usage| {
            let remaining = match usage.name {
                "Claude" => remaining(usage.limits.iter().filter(|limit| is_hourly(limit))),
                _ => usage.remaining_percent(),
            }?;
            Some((usage.name, remaining))
        })
        .collect();
    if readings.is_empty() {
        return "Agent quota unavailable".to_string();
    }
    let parts: Vec<_> = readings
        .into_iter()
        .filter(|(_, remaining)| *remaining > 0)
        .map(|(name, remaining)| format!("{name} {remaining}%"))
        .collect();
    parts.join(", ")
}

/// One provider's short- and long-window rows for the expanded tray menu.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// One metered window, kept numeric so the menu bar can draw it as a meter
/// rather than as text pretending to be one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meter {
    pub label: String,
    pub remaining_percent: u64,
    pub resets_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeterGroup {
    pub provider: &'static str,
    pub meters: Vec<Meter>,
}

/// The same grouping as `quota_groups` — Spark separated from Codex, short
/// windows before long ones — without flattening the numbers into strings.
pub fn meter_groups(usages: &[Usage]) -> Vec<MeterGroup> {
    let mut groups = Vec::new();
    for usage in usages {
        let sections: Vec<(&'static str, Vec<&Limit>)> = if usage.name == "Codex" {
            vec![
                ("Codex", usage.limits.iter().filter(|l| !is_spark(l)).collect()),
                ("Codex Spark", usage.limits.iter().filter(|l| is_spark(l)).collect()),
            ]
        } else {
            vec![(usage.name, usage.limits.iter().collect())]
        };
        for (provider, mut limits) in sections {
            if limits.is_empty() {
                continue;
            }
            limits.sort_by_key(|limit| !is_hourly(limit));
            let meters: Vec<Meter> = limits
                .into_iter()
                .map(|limit| Meter {
                    label: meter_label(limit),
                    remaining_percent: limit.remaining_percent(),
                    resets_at: limit.resets_at,
                })
                .collect();
            groups.push(MeterGroup { provider, meters });
        }
    }
    groups
}

fn meter_label(limit: &Limit) -> String {
    let label = limit.label.strip_prefix("Spark ").unwrap_or(&limit.label);
    let mut characters = label.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => "Window".into(),
    }
}

/// When a window refills, phrased for a glance: a countdown while it is the
/// same day's business, a weekday and time once it is further away.
pub fn reset_label(resets_at: Option<u64>, now: u64) -> String {
    let Some(resets_at) = resets_at else {
        return String::new();
    };
    let remaining = resets_at.saturating_sub(now);
    match remaining {
        0..=59 => "now".into(),
        60..=3_599 => format!("in {}m", remaining / 60),
        3_600..=86_399 => format!("in {}h {}m", remaining / 3_600, remaining % 3_600 / 60),
        _ => timestamp_local(resets_at)
            .map(|reset| reset.format("%a %H:%M").to_string())
            .unwrap_or_default(),
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

/// Query the enabled providers, keeping failures alongside successes so one
/// missing CLI never hides the other's numbers. A provider the user switched
/// off is never contacted and never reported as an error.
pub fn fetch_enabled(providers: &config::ProviderConfig) -> (Vec<Usage>, Vec<String>) {
    let mut usages = Vec::new();
    let mut errors = Vec::new();

    for (name, enabled) in [
        ("Codex", providers.codex),
        ("Claude", providers.claude),
        ("Cursor", providers.cursor),
    ] {
        if !enabled {
            continue;
        }
        let result = match name {
            "Codex" => codex::fetch(),
            "Claude" => claude::fetch(),
            _ => cursor::fetch(),
        };
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
    use super::{meter_groups, ordinal_suffix, reset_label, summary, tray_summary, Limit, Usage};

    #[test]
    fn meters_keep_numbers_and_put_short_windows_first() {
        let limit = |label: &str, used: f64| Limit {
            label: label.into(),
            used_percent: used,
            resets_at: None,
        };
        let groups = meter_groups(&[Usage {
            name: "Codex",
            limits: vec![limit("Weekly", 82.0), limit("Spark 5-hour", 0.0), limit("5-hour", 10.0)],
        }]);
        assert_eq!(groups[0].provider, "Codex");
        assert_eq!(groups[0].meters[0].label, "5-hour");
        assert_eq!(groups[0].meters[1].remaining_percent, 18);
        assert_eq!(groups[1].provider, "Codex Spark");
        assert_eq!(groups[1].meters[0].label, "5-hour");
    }

    #[test]
    fn exhausted_windows_stay_in_the_panel_and_leave_the_menu_bar() {
        let limit = |label: &str, used: f64| Limit {
            label: label.into(),
            used_percent: used,
            resets_at: None,
        };
        let usages = [Usage {
            name: "Claude",
            limits: vec![limit("5-hour", 100.0), limit("Weekly", 40.0)],
        }];
        assert_eq!(tray_summary(&usages), "");
        assert_eq!(summary(&usages), "Claude 0%");
        let groups = meter_groups(&usages);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].meters.len(), 2);
        assert_eq!(groups[0].meters[0].remaining_percent, 0);
        assert_eq!(groups[0].meters[1].label, "Weekly");
    }

    #[test]
    fn reset_labels_count_down_while_the_window_is_close() {
        assert_eq!(reset_label(None, 0), "");
        assert_eq!(reset_label(Some(30), 0), "now");
        assert_eq!(reset_label(Some(42 * 60), 0), "in 42m");
        assert_eq!(reset_label(Some(2 * 3_600 + 5 * 60), 0), "in 2h 5m");
    }

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
