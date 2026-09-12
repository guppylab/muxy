use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use muxy_server_core::ServerSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    default_shell: Option<PathBuf>,
    history_budget_bytes: u64,
    shell_integration: bool,
}

impl Default for SettingsFile {
    fn default() -> Self {
        let defaults = ServerSettings::default();
        Self {
            default_shell: defaults.default_shell,
            shell_integration: defaults.shell_integration,
            history_budget_bytes: defaults.history_budget_bytes,
        }
    }
}

pub(crate) fn load(path: &Path) -> io::Result<ServerSettings> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let defaults =
                toml::to_string_pretty(&SettingsFile::default()).map_err(io::Error::other)?;
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return load(path),
                Err(error) => return Err(error),
            };
            file.write_all(defaults.as_bytes())?;
            defaults
        }
        Err(error) => return Err(error),
    };
    let settings: SettingsFile = toml::from_str(&contents).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })?;
    let settings = ServerSettings {
        default_shell: settings.default_shell,
        shell_integration: settings.shell_integration,
        history_budget_bytes: settings.history_budget_bytes,
    };
    settings.document().validate().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid server settings: {error:?}"),
        )
    })?;
    Ok(settings)
}

pub(crate) fn save(path: &Path, settings: &ServerSettings) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let source = toml::to_string_pretty(&SettingsFile {
        default_shell: settings.default_shell.clone(),
        history_budget_bytes: settings.history_budget_bytes,
        shell_integration: settings.shell_integration,
    })
    .map_err(io::Error::other)?;
    let temporary = path.with_file_name(format!(
        ".server-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_settings_enable_integration_and_explicit_false_survives_serialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let existing: SettingsFile =
            toml::from_str("default_shell = '/bin/zsh'\nhistory_budget_bytes = 4096")?;
        assert!(existing.shell_integration);
        let disabled: SettingsFile = toml::from_str("shell_integration = false")?;
        assert!(!disabled.shell_integration);
        let saved: SettingsFile = toml::from_str(&toml::to_string(&disabled)?)?;
        assert!(!saved.shell_integration);
        Ok(())
    }
}
