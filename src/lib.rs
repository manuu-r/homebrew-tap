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

/// One agent's quota.
pub struct Usage {
    pub name: &'static str,
    pub limits: Vec<Limit>,
}

impl Usage {
    /// Percentage left in the tightest window, which is the only one that can
    /// actually stop you.
    pub fn remaining_percent(&self) -> Option<u64> {
        let used = self
            .limits
            .iter()
            .map(|limit| limit.used_percent)
            .max_by(f64::total_cmp)?;
        Some((100.0 - used).clamp(0.0, 100.0).round() as u64)
    }
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
        false => format!("{}", parts.join(", ")),
    }
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
