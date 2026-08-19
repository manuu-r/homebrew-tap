//! Reads how much of each agent's quota is left.
//!
//! Each provider talks to its own vendor tooling and hands back the windows it
//! meters; everything downstream is just formatting.

pub mod claude;
pub mod codex;

use std::time::{SystemTime, UNIX_EPOCH};

/// One rate-limit window, such as Codex's weekly quota.
pub struct Limit {
    pub label: String,
    pub used_percent: f64,
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

/// Provider groups and compact per-window rows for the expanded tray menu.
pub fn quota_groups(usages: &[Usage]) -> Vec<(&'static str, Vec<String>)> {
    let label_width = usages
        .iter()
        .flat_map(|usage| &usage.limits)
        .map(|limit| tray_label(&limit.label).chars().count())
        .max()
        .unwrap_or(0);

    usages
        .iter()
        .filter(|usage| !usage.limits.is_empty())
        .map(|usage| {
            let rows = usage
                .limits
                .iter()
                .map(|limit| {
                    let label = tray_label(&limit.label);
                    let remaining = limit.remaining_percent();
                    let padding = "\u{2006}".repeat(label_width - label.chars().count());
                    format!(
                        "  {}{}  {}  {remaining:>3}%",
                        label,
                        padding,
                        quota_bar(remaining)
                    )
                })
                .collect();
            (usage.name, rows)
        })
        .collect()
}

fn tray_label(label: &str) -> &str {
    match label {
        "Spark Weekly" => "Spark",
        _ => label,
    }
}

fn quota_bar(percent: u64) -> String {
    const SEGMENTS: usize = 8;
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
