use super::keys::{KeyCombo, canonical_key, legacy_virtual_key_code};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum QuickTerminalShortcut {
    #[serde(rename = "unassigned")]
    Unassigned,
    #[serde(rename = "keyCombo")]
    KeyCombo {
        #[serde(rename = "keyCombo")]
        key_combo: KeyCombo,
        #[serde(rename = "virtualKeyCode")]
        virtual_key_code: u16,
    },
}

impl<'de> Deserialize<'de> for QuickTerminalShortcut {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "type")]
        enum StoredShortcut {
            #[serde(rename = "unassigned", alias = "doubleShift")]
            Unassigned,
            #[serde(rename = "keyCombo")]
            KeyCombo {
                #[serde(rename = "keyCombo")]
                key_combo: KeyCombo,
                #[serde(default, rename = "virtualKeyCode")]
                virtual_key_code: Option<u16>,
            },
        }

        match StoredShortcut::deserialize(deserializer)? {
            StoredShortcut::Unassigned => Ok(Self::Unassigned),
            StoredShortcut::KeyCombo {
                key_combo,
                virtual_key_code,
            } => {
                let virtual_key_code = virtual_key_code
                    .or_else(|| legacy_virtual_key_code(&key_combo.key))
                    .ok_or_else(|| serde::de::Error::custom("unsupported virtual key code"))?;
                Ok(Self::KeyCombo {
                    key_combo,
                    virtual_key_code,
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistrationIdentity {
    pub modifiers: u64,
    pub virtual_key_code: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictCandidate {
    pub label: String,
    pub combo: KeyCombo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutConflict {
    pub label: String,
}

impl QuickTerminalShortcut {
    pub fn canonicalized(
        &self,
        mut key_resolver: impl FnMut(u16) -> Option<String>,
    ) -> Option<Self> {
        match self {
            Self::Unassigned => Some(Self::Unassigned),
            Self::KeyCombo {
                virtual_key_code, ..
            } => {
                let identity = self.registration_identity()?;
                let key = canonical_key(&key_resolver(*virtual_key_code)?);
                let key_combo = KeyCombo::new(&key, identity.modifiers);
                key_combo.is_supported_shortcut().then_some(Self::KeyCombo {
                    key_combo,
                    virtual_key_code: *virtual_key_code,
                })
            }
        }
    }

    pub fn registration_identity(&self) -> Option<RegistrationIdentity> {
        let Self::KeyCombo {
            key_combo,
            virtual_key_code,
        } = self
        else {
            return None;
        };
        (key_combo.is_supported_shortcut() && *virtual_key_code <= 127).then_some(
            RegistrationIdentity {
                modifiers: key_combo.modifiers,
                virtual_key_code: *virtual_key_code,
            },
        )
    }

    pub fn key_combo(&self) -> Option<&KeyCombo> {
        match self {
            Self::KeyCombo { key_combo, .. } => Some(key_combo),
            Self::Unassigned => None,
        }
    }

    pub fn find_conflict(
        &self,
        candidates: &[ConflictCandidate],
        key_resolver: impl FnMut(u16) -> Option<String>,
    ) -> Option<ShortcutConflict> {
        let shortcut = self.canonicalized(key_resolver)?;
        let combo = shortcut.key_combo()?;
        candidates
            .iter()
            .find(|candidate| combo.conflicts_with(&candidate.combo))
            .map(|candidate| ShortcutConflict {
                label: candidate.label.clone(),
            })
    }
}
