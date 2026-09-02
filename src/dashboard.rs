//! One immutable application snapshot shared by the menu bar and every paired
//! accessory. Device requests never trigger provider or Calendar fetches
//! themselves.

use serde_json::{json, Value};

use crate::{
    calendar::{self, CalendarEvent},
    config::{self, Config},
    fetch_all, now_seconds, summary, Usage,
};

#[derive(Clone)]
pub struct DashboardSnapshot {
    pub generated_at: u64,
    pub config: Config,
    pub settings_error: Option<String>,
    pub usages: Vec<Usage>,
    pub quota_errors: Vec<String>,
    pub calendar_events: Vec<CalendarEvent>,
    pub calendar_error: Option<String>,
}

impl DashboardSnapshot {
    pub fn collect() -> Self {
        let (config, settings_error) = match config::load_or_create() {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(error)),
        };
        let (usages, quota_errors) = fetch_all();
        let (calendar_events, calendar_error) = match calendar::fetch(&config.calendar) {
            Ok(events) => (events, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self {
            generated_at: now_seconds(),
            config,
            settings_error,
            usages,
            quota_errors,
            calendar_events,
            calendar_error,
        }
    }

    pub fn json(&self) -> Value {
        let providers = self
            .usages
            .iter()
            .map(|usage| {
                json!({
                    "name": usage.name,
                    "remaining_percent": usage.remaining_percent(),
                    "limits": usage.limits.iter().map(|limit| json!({
                        "label": limit.label,
                        "used_percent": limit.used_percent,
                        "remaining_percent": limit.remaining_percent(),
                        "resets_at": limit.resets_at,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        let events = self
            .calendar_events
            .iter()
            .map(|event| {
                json!({
                    "title": event.title,
                    "starts_at": event.starts_at as i64,
                    "ends_at": event.ends_at as i64,
                    "all_day": event.all_day,
                })
            })
            .collect::<Vec<_>>();
        let todos = self
            .config
            .todos
            .iter()
            .enumerate()
            .map(|(id, todo)| {
                json!({
                    "id": id,
                    "title": todo.title,
                    "completed": todo.completed,
                })
            })
            .collect::<Vec<_>>();

        json!({
            "protocol": "dev.gauge.dashboard",
            "schema_version": 1,
            "generated_at": self.generated_at,
            "refresh_seconds": self.config.refresh_seconds,
            "quota": {
                "summary": summary(&self.usages),
                "providers": providers,
                "errors": self.quota_errors,
            },
            "calendar": {
                "enabled": self.config.calendar.enabled,
                "events": events,
                "error": self.calendar_error,
            },
            "todos": todos,
            "errors": {
                "settings": self.settings_error,
            },
        })
    }

    pub fn json_string(&self) -> String {
        self.json().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Todo, Limit};

    #[test]
    fn preserves_the_version_one_edge_schema() {
        let mut config = Config::default();
        config.todos.push(Todo {
            title: "Ship accessory firmware".into(),
            completed: false,
        });
        let snapshot = DashboardSnapshot {
            generated_at: 42,
            config,
            settings_error: None,
            usages: vec![Usage {
                name: "Codex",
                limits: vec![Limit {
                    label: "Weekly".into(),
                    used_percent: 25.0,
                    resets_at: Some(99),
                }],
            }],
            quota_errors: Vec::new(),
            calendar_events: Vec::new(),
            calendar_error: None,
        };
        let json = snapshot.json();
        assert_eq!(json["protocol"], "dev.gauge.dashboard");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["quota"]["providers"][0]["remaining_percent"], 75);
        assert_eq!(json["todos"][0]["title"], "Ship accessory firmware");
    }
}
