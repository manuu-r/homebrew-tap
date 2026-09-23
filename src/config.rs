use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

const CONFIG_FILE: &str = "config.json";
pub const REFRESH_CHOICES: [u64; 4] = [60, 120, 300, 900];

/// User-controlled settings for the optional menu-bar sections.
///
/// An empty calendar name list means every calendar available through macOS
/// Calendar. The configuration intentionally uses names rather than opaque
/// EventKit IDs so it can be edited without a separate setup screen.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub refresh_seconds: u64,
    pub providers: ProviderConfig,
    pub calendar: CalendarConfig,
    pub tasks: TasksConfig,
    pub todos: Vec<Todo>,
    pub accessories: AccessoryConfig,
}

/// Which agents Gauge asks about. Turning one off removes it from the menu-bar
/// title, the popover, and every paired accessory, and skips its network call.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderConfig {
    #[serde(default = "enabled")]
    pub codex: bool,
    #[serde(default = "enabled")]
    pub claude: bool,
    /// Missing from older settings files, which predate this provider.
    #[serde(default = "enabled")]
    pub cursor: bool,
}

fn enabled() -> bool {
    true
}

/// Optional local-network extension for paired displays and future Gauge
/// accessories. Pairing controls this setting; users never manage ports or
/// bearer tokens themselves.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AccessoryConfig {
    pub enabled: bool,
    pub port: u16,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub enabled: bool,
    pub calendar_names: Vec<String>,
    pub max_events: usize,
    pub look_ahead_hours: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct TasksConfig {
    pub enabled: bool,
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
            providers: ProviderConfig::default(),
            calendar: CalendarConfig::default(),
            tasks: TasksConfig::default(),
            todos: Vec::new(),
            accessories: AccessoryConfig::default(),
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            codex: true,
            claude: true,
            cursor: true,
        }
    }
}

impl Default for AccessoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 45_831,
            display_name: "Gauge on this Mac".into(),
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

impl Default for TasksConfig {
    fn default() -> Self {
        Self { enabled: true }
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
            let config: Config = serde_json::from_str(&contents)
                .map_err(|error| format!("invalid settings at {}: {error}", path.display()))?;
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

/// Apply one edit and persist it. Every settings control funnels through this,
/// so a toggle can never write a partially updated file.
pub fn update(edit: impl FnOnce(&mut Config)) -> Result<Config, String> {
    let mut config = load_or_create()?;
    edit(&mut config);
    config.refresh_seconds = config.refresh_seconds.clamp(30, 3_600);
    save(&config)?;
    Ok(config)
}

pub fn set_accessories_enabled(enabled: bool) -> Result<Config, String> {
    update(|config| config.accessories.enabled = enabled)
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
