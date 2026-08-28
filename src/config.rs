use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

const CONFIG_FILE: &str = "config.json";

/// User-controlled settings for the optional menu-bar sections.
///
/// An empty calendar name list means every calendar available through macOS
/// Calendar. The configuration intentionally uses names rather than opaque
/// EventKit IDs so it can be edited without a separate setup screen.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub refresh_seconds: u64,
    pub calendar: CalendarConfig,
    pub stocks: StockConfig,
    pub todos: Vec<Todo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub enabled: bool,
    pub calendar_names: Vec<String>,
    pub max_events: usize,
    pub look_ahead_hours: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct StockConfig {
    pub enabled: bool,
    pub symbols: Vec<String>,
    pub max_items: usize,
    /// A Yahoo Finance chart-compatible URL template. `{symbol}` is replaced
    /// with each configured symbol. Keeping this in settings allows a private
    /// endpoint or proxy to be used without rebuilding Gauge.
    pub quote_url_template: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Todo {
    pub title: String,
    #[serde(default)]
    pub completed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_seconds: 120,
            calendar: CalendarConfig::default(),
            stocks: StockConfig::default(),
            todos: Vec::new(),
        }
    }
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            calendar_names: Vec::new(),
            max_events: 1,
            look_ahead_hours: 24,
        }
    }
}

impl Default for StockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            symbols: vec!["^NSEI".into(), "^BSESN".into()],
            max_items: 4,
            quote_url_template:
                "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?range=1d&interval=1m"
                    .into(),
        }
    }
}

pub fn path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("could not determine the home directory")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Gauge")
        .join(CONFIG_FILE))
}

/// Load settings, creating a readable default file the first time Gauge runs.
pub fn load_or_create() -> Result<Config, String> {
    let path = path()?;
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let mut config: Config = serde_json::from_str(&contents)
                .map_err(|error| format!("invalid settings at {}: {error}", path.display()))?;
            if normalize(&mut config) {
                save(&config)?;
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            save(&config)?;
            Ok(config)
        }
        Err(error) => Err(format!(
            "could not read settings at {}: {error}",
            path.display()
        )),
    }
}

/// Existing configurations from before the default ticker set are upgraded
/// only when their stock section is enabled but has no symbols. Disabling the
/// section remains the explicit way to opt out of those defaults.
fn normalize(config: &mut Config) -> bool {
    if config.stocks.enabled && config.stocks.symbols.is_empty() {
        config.stocks.symbols = StockConfig::default().symbols;
        return true;
    }
    false
}

pub fn save(config: &Config) -> Result<(), String> {
    let path = path()?;
    let directory = path
        .parent()
        .ok_or("settings path has no containing directory")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create settings directory: {error}"))?;
    let body = serde_json::to_string_pretty(config)
        .map_err(|error| format!("could not encode settings: {error}"))?;
    fs::write(&path, format!("{body}\n"))
        .map_err(|error| format!("could not write settings at {}: {error}", path.display()))
}

pub fn toggle_todo(index: usize) -> Result<(), String> {
    let mut config = load_or_create()?;
    let todo = config
        .todos
        .get_mut(index)
        .ok_or("that to-do no longer exists")?;
    todo.completed = !todo.completed;
    save(&config)
}

pub fn delete_todo(index: usize) -> Result<(), String> {
    let mut config = load_or_create()?;
    if index >= config.todos.len() {
        return Err("that to-do no longer exists".into());
    }
    config.todos.remove(index);
    save(&config)
}

pub fn update_todo(index: usize, title: impl Into<String>) -> Result<(), String> {
    let title = title.into();
    let title = title.trim();
    if title.is_empty() {
        return Ok(());
    }
    let mut config = load_or_create()?;
    let todo = config
        .todos
        .get_mut(index)
        .ok_or("that to-do no longer exists")?;
    todo.title = title.into();
    save(&config)
}

pub fn todo_is_completed(index: usize) -> Result<bool, String> {
    let config = load_or_create()?;
    config
        .todos
        .get(index)
        .map(|todo| todo.completed)
        .ok_or_else(|| "that to-do no longer exists".into())
}

pub fn add_todo(title: impl Into<String>) -> Result<(), String> {
    let title = title.into();
    let title = title.trim();
    if title.is_empty() {
        return Ok(());
    }
    let mut config = load_or_create()?;
    config.todos.push(Todo {
        title: title.into(),
        completed: false,
    });
    save(&config)
}

#[cfg(test)]
mod tests {
    use super::{normalize, Config};

    #[test]
    fn default_configuration_prioritizes_indian_market_indices() {
        let config = Config::default();
        assert!(config.calendar.enabled);
        assert!(config.stocks.enabled);
        assert_eq!(config.stocks.symbols, ["^NSEI", "^BSESN"]);
    }

    #[test]
    fn upgrades_an_enabled_empty_stock_section_to_the_default_indices() {
        let mut config = Config::default();
        config.stocks.symbols.clear();
        assert!(normalize(&mut config));
        assert_eq!(config.stocks.symbols, ["^NSEI", "^BSESN"]);
    }
}
