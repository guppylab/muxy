use serde::{Deserialize, Serialize};

pub const SHIFT: u64 = 1 << 17;
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const COMMAND: u64 = 1 << 20;
pub const SUPPORTED_MODIFIER_MASK: u64 = SHIFT | CONTROL | OPTION | COMMAND;
pub const CONVENTIONAL_MODIFIER_MASK: u64 = CONTROL | OPTION | COMMAND;

pub fn canonical_modifiers(modifiers: u64) -> u64 {
    modifiers & SUPPORTED_MODIFIER_MASK
}

pub fn canonical_key(key: &str) -> String {
    let lower = key.to_lowercase();
    match lower.as_str() {
        " " => "space".to_owned(),
        "left" | "\u{f702}" => "leftarrow".to_owned(),
        "right" | "\u{f703}" => "rightarrow".to_owned(),
        "up" | "\u{f700}" => "uparrow".to_owned(),
        "down" | "\u{f701}" => "downarrow".to_owned(),
        "enter" => "return".to_owned(),
        _ => lower,
    }
}

pub fn supported_shortcut_key(key: &str) -> bool {
    matches!(
        key,
        "leftarrow" | "rightarrow" | "uparrow" | "downarrow" | "tab" | "return" | "space"
    ) || {
        let mut characters = key.chars();
        characters.next().is_some() && characters.next().is_none()
    }
}

pub fn legacy_key_for_virtual_key_code(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "a",
        1 => "s",
        2 => "d",
        3 => "f",
        4 => "h",
        5 => "g",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        11 => "b",
        12 => "q",
        13 => "w",
        14 => "e",
        15 => "r",
        16 => "y",
        17 => "t",
        18 | 83 => "1",
        19 | 84 => "2",
        20 | 85 => "3",
        21 | 86 => "4",
        22 | 88 => "6",
        23 | 87 => "5",
        24 | 81 => "=",
        25 | 92 => "9",
        26 | 89 => "7",
        27 | 78 => "-",
        28 | 91 => "8",
        29 | 82 => "0",
        30 => "]",
        31 => "o",
        32 => "u",
        33 => "[",
        34 => "i",
        35 => "p",
        36 | 76 => "return",
        37 => "l",
        38 => "j",
        39 => "'",
        40 => "k",
        41 => ";",
        42 => "\\",
        43 => ",",
        44 | 75 => "/",
        45 => "n",
        46 => "m",
        47 | 65 => ".",
        49 => "space",
        50 => "`",
        67 => "*",
        69 => "+",
        123 => "leftarrow",
        124 => "rightarrow",
        125 => "downarrow",
        126 => "uparrow",
        48 => "tab",
        _ => return None,
    })
}

pub fn legacy_virtual_key_code(key: &str) -> Option<u16> {
    let key = canonical_key(key);
    (0..=127).find(|code| legacy_key_for_virtual_key_code(*code) == Some(key.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct KeyCombo {
    pub key: String,
    pub modifiers: u64,
}

impl KeyCombo {
    pub fn new(key: &str, modifiers: u64) -> Self {
        Self {
            key: key.to_owned(),
            modifiers,
        }
    }

    pub fn is_assigned(&self) -> bool {
        !self.key.is_empty()
    }

    #[must_use]
    pub fn canonicalized(&self) -> Self {
        Self {
            key: canonical_key(&self.key),
            modifiers: canonical_modifiers(self.modifiers),
        }
    }

    pub fn is_canonical(&self) -> bool {
        self.key == canonical_key(&self.key)
            && self.modifiers == canonical_modifiers(self.modifiers)
    }

    pub fn has_conventional_modifier(&self) -> bool {
        self.modifiers & CONVENTIONAL_MODIFIER_MASK != 0
    }

    pub fn is_supported_shortcut(&self) -> bool {
        self.is_assigned()
            && self.is_canonical()
            && self.has_conventional_modifier()
            && supported_shortcut_key(&self.key)
    }

    pub fn keystroke(&self) -> Option<String> {
        if !self.is_assigned() {
            return None;
        }
        let mut parts = Vec::new();
        if self.modifiers & CONTROL != 0 {
            parts.push("ctrl");
        }
        if self.modifiers & OPTION != 0 {
            parts.push("alt");
        }
        if self.modifiers & SHIFT != 0 {
            parts.push("shift");
        }
        if self.modifiers & COMMAND != 0 {
            parts.push(if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            });
        }
        parts.push(match self.key.as_str() {
            "leftarrow" => "left",
            "rightarrow" => "right",
            "uparrow" => "up",
            "downarrow" => "down",
            "return" => "enter",
            key => key,
        });
        Some(parts.join("-"))
    }

    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.canonicalized()
            .keystroke()
            .zip(other.canonicalized().keystroke())
            .is_some_and(|(left, right)| left == right)
    }

    pub fn display(&self) -> String {
        if !self.is_assigned() {
            return "Unassigned".to_owned();
        }
        if cfg!(target_os = "macos") {
            let mut value = String::new();
            if self.modifiers & CONTROL != 0 {
                value.push('⌃');
            }
            if self.modifiers & OPTION != 0 {
                value.push('⌥');
            }
            if self.modifiers & SHIFT != 0 {
                value.push('⇧');
            }
            if self.modifiers & COMMAND != 0 {
                value.push('⌘');
            }
            value.push_str(match self.key.as_str() {
                "leftarrow" => "←",
                "rightarrow" => "→",
                "uparrow" => "↑",
                "downarrow" => "↓",
                "tab" => "⇥",
                "return" => "↩",
                key => key,
            });
            return value;
        }
        self.keystroke().unwrap_or_default()
    }
}
