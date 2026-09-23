//! Paths persisted in login items and hooks must survive Homebrew upgrades.

use std::path::{Path, PathBuf};

pub fn persistent_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the Gauge executable: {error}"))?;
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("could not resolve the Gauge executable: {error}"))?;
    persistent_path(&executable)
}

fn persistent_path(executable: &Path) -> Result<PathBuf, String> {
    let Some(stable) = homebrew_path(executable) else {
        return Ok(executable.to_path_buf());
    };
    // Do not persist a guessed path, or silently register a different install.
    if stable.canonicalize().ok().as_ref() != Some(&executable.to_path_buf()) {
        return Err(
            "Homebrew's opt/gauge link does not point to this Gauge installation; \
                    run the current Homebrew installation before enabling login or hooks"
                .into(),
        );
    }
    Ok(stable)
}

fn homebrew_path(executable: &Path) -> Option<PathBuf> {
    for version in executable.ancestors().skip(1) {
        let Some(rack) = version.parent() else {
            continue;
        };
        let Some(cellar) = rack.parent() else {
            continue;
        };
        if rack.file_name()? == "gauge" && cellar.file_name()? == "Cellar" {
            let relative = executable.strip_prefix(version).ok()?;
            return Some(cellar.parent()?.join("opt/gauge").join(relative));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_homebrew_prefixes_without_changing_the_bundle_layout() {
        for prefix in ["/opt/homebrew", "/usr/local", "/custom/brew"] {
            for suffix in ["Gauge.app/Contents/MacOS/gauge", "bin/gauge"] {
                let original = PathBuf::from(format!("{prefix}/Cellar/gauge/0.2.0/{suffix}"));
                assert_eq!(
                    homebrew_path(&original),
                    Some(PathBuf::from(format!("{prefix}/opt/gauge/{suffix}")))
                );
            }
        }
    }

    #[test]
    fn keeps_standalone_and_development_paths() {
        for path in [
            "/Applications/Gauge.app/Contents/MacOS/gauge",
            "/tmp/Gauge/target/debug/gauge",
            "/tmp/Cellar/another/1/bin/gauge",
        ] {
            assert_eq!(
                persistent_path(Path::new(path)).unwrap(),
                PathBuf::from(path)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn validates_opt_link_and_survives_removal_of_the_old_version() {
        use std::{fs, os::unix::fs::symlink};
        let root = std::env::temp_dir().join(format!(
            "gauge-executable-{}-{}",
            std::process::id(),
            crate::now_seconds()
        ));
        fs::create_dir_all(root.join("opt")).unwrap();
        let root = root.canonicalize().unwrap();
        let old = root.join("Cellar/gauge/0.2.0/bin/gauge");
        let new = root.join("Cellar/gauge/0.2.1/bin/gauge");
        for path in [&old, &new] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"gauge").unwrap();
        }
        assert!(persistent_path(&old).is_err());
        let link = root.join("opt/gauge");
        symlink(root.join("Cellar/gauge/0.2.0"), &link).unwrap();
        let saved = persistent_path(&old).unwrap();
        assert_eq!(saved, link.join("bin/gauge"));
        fs::remove_file(&link).unwrap();
        symlink(root.join("Cellar/gauge/0.2.1"), &link).unwrap();
        assert!(persistent_path(&old).is_err());
        fs::remove_file(&old).unwrap();
        assert_eq!(saved.canonicalize().unwrap(), new);
        fs::remove_dir_all(root).unwrap();
    }
}
