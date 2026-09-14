use std::path::Path;

use muxy_core::quick_terminal::QuickTerminalShortcut;
use serde::{Deserialize, Serialize};

use crate::settings::{Error, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuickTerminalSettings {
    pub enabled: bool,
    pub width: u16,
    pub height: u16,
    pub transparency: u8,
    pub blur: u8,
    pub shortcut: QuickTerminalShortcut,
}

impl Default for QuickTerminalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 720,
            height: 430,
            transparency: 18,
            blur: 70,
            shortcut: QuickTerminalShortcut::Unassigned,
        }
    }
}

impl QuickTerminalSettings {
    pub fn validate(&self) -> Result<()> {
        for (name, valid) in [
            ("width", (480..=1200).contains(&self.width)),
            ("height", (280..=800).contains(&self.height)),
            ("transparency", self.transparency <= 55),
            ("blur", self.blur <= 100),
            (
                "shortcut",
                !matches!(self.shortcut, QuickTerminalShortcut::KeyCombo { .. })
                    || self.shortcut.registration_identity().is_some(),
            ),
        ] {
            if !valid {
                return Err(Error::new(
                    format!("quick_terminal.{name}"),
                    "invalid value",
                ));
            }
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        crate::settings::appearance::save_section(path, "quick_terminal", self)
            .map_err(|error| Error::new("quick_terminal", error))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn settings_preserve_shortcut_and_enforce_original_ranges() {
        let mut settings = QuickTerminalSettings {
            shortcut: QuickTerminalShortcut::DoubleShift,
            ..Default::default()
        };
        let encoded = toml::to_string(&settings).unwrap();
        assert_eq!(
            toml::from_str::<QuickTerminalSettings>(&encoded).unwrap(),
            settings
        );
        for width in [0, 479, 1201, u16::MAX] {
            settings.width = width;
            assert!(settings.validate().is_err());
        }
    }
}
