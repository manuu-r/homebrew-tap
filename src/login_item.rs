//! The "Open at login" switch, backed by a plain LaunchAgent.
//!
//! The presence of the plist is the only state; there is nothing to keep in
//! sync with `config.json`, so the switch can never disagree with what macOS
//! will actually do at the next login.

use std::{env, fs, path::PathBuf};

const LABEL: &str = "dev.gauge.tray";

fn plist_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("could not determine the home directory")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

pub fn is_enabled() -> bool {
    plist_path().is_ok_and(|path| path.exists())
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let path = plist_path()?;
    if !enabled {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not turn off Open at Login: {error}")),
        };
    }

    let executable = gauge::executable::persistent_executable()?;
    let executable = executable
        .to_str()
        .ok_or("the Gauge executable path is not valid UTF-8")?;
    let directory = path
        .parent()
        .ok_or("the LaunchAgents path has no containing directory")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create the LaunchAgents directory: {error}"))?;
    fs::write(&path, agent_plist(executable))
        .map_err(|error| format!("could not turn on Open at Login: {error}"))
}

fn agent_plist(executable: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--tray</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        escape(executable)
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_paths_that_would_break_the_plist() {
        let plist = agent_plist("/Users/a&b/Gauge.app/Contents/MacOS/gauge");
        assert!(plist.contains("/Users/a&amp;b/Gauge.app/Contents/MacOS/gauge"));
        assert!(plist.contains("<string>--tray</string>"));
    }
}
