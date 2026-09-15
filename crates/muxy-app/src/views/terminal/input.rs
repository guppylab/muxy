use gpui::{Keystroke, Modifiers};
use muxy_protocol::Modes;

pub(crate) fn uses_text_input(key: &Keystroke, option_as_alt: bool) -> bool {
    !key.modifiers.platform
        && !key.modifiers.control
        && (!key.modifiers.alt || !option_as_alt)
        && key
            .key_char
            .as_deref()
            .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

#[cfg(test)]
pub(crate) fn encode(key: &Keystroke, modes: Modes, option_as_alt: bool) -> Option<Vec<u8>> {
    encode_with_bindings(key, modes, option_as_alt, true)
}

pub(crate) fn encode_with_bindings(
    key: &Keystroke,
    modes: Modes,
    option_as_alt: bool,
    defaults: bool,
) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    if modifiers.platform {
        return None;
    }
    if modifiers.alt && !option_as_alt && uses_text_input(key, option_as_alt) {
        return Some(key.key_char.as_deref()?.as_bytes().to_vec());
    }
    let modifier = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let sequence = match key.key.as_str() {
        "left" if cfg!(target_os = "macos") && defaults && modifier == 3 => Some("\x1bb".into()),
        "right" if cfg!(target_os = "macos") && defaults && modifier == 3 => Some("\x1bf".into()),
        "up" => Some(cursor('A', modes, modifier)),
        "down" => Some(cursor('B', modes, modifier)),
        "right" => Some(cursor('C', modes, modifier)),
        "left" => Some(cursor('D', modes, modifier)),
        "home" => Some(cursor('H', modes, modifier)),
        "end" => Some(cursor('F', modes, modifier)),
        "pageup" => Some(function(5, modifier)),
        "pagedown" => Some(function(6, modifier)),
        "insert" => Some(function(2, modifier)),
        "delete" => Some(function(3, modifier)),
        "f1" | "f2" | "f3" | "f4" => {
            let suffix = match key.key.as_str() {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            Some(if modifier == 1 {
                format!("\x1bO{suffix}")
            } else {
                format!("\x1b[1;{modifier}{suffix}")
            })
        }
        "f5" => Some(function(15, modifier)),
        "f6" => Some(function(17, modifier)),
        "f7" => Some(function(18, modifier)),
        "f8" => Some(function(19, modifier)),
        "f9" => Some(function(20, modifier)),
        "f10" => Some(function(21, modifier)),
        "f11" => Some(function(23, modifier)),
        "f12" => Some(function(24, modifier)),
        "f13" => Some(function(25, modifier)),
        "f14" => Some(function(26, modifier)),
        "f15" => Some(function(28, modifier)),
        "f16" => Some(function(29, modifier)),
        "f17" => Some(function(31, modifier)),
        "f18" => Some(function(32, modifier)),
        "f19" => Some(function(33, modifier)),
        "f20" => Some(function(34, modifier)),
        _ => None,
    };
    if let Some(sequence) = sequence {
        return Some(sequence.into_bytes());
    }
    let bytes = match key.key.as_str() {
        "enter" | "return" => b"\r".to_vec(),
        "backspace" if modifiers.control => vec![0x08],
        "backspace" => vec![0x7f],
        "escape" => vec![0x1b],
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        _ if modifiers.control => vec![control(&key.key)?],
        "space" => vec![b' '],
        _ if modifiers.alt && key.key.chars().count() == 1 => {
            if modifiers.shift {
                key.key.to_uppercase().into_bytes()
            } else {
                key.key.as_bytes().to_vec()
            }
        }
        _ => {
            let text = key.key_char.as_deref().filter(|text| !text.is_empty())?;
            if text.chars().any(char::is_control) {
                return None;
            }
            text.as_bytes().to_vec()
        }
    };
    Some(prefix_alt(bytes, modifiers))
}

fn prefix_alt(mut bytes: Vec<u8>, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn cursor(suffix: char, modes: Modes, modifier: u8) -> String {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{suffix}")
    } else if modes.application_cursor_keys {
        format!("\x1bO{suffix}")
    } else {
        format!("\x1b[{suffix}")
    }
}

fn function(number: u8, modifier: u8) -> String {
    if modifier > 1 {
        format!("\x1b[{number};{modifier}~")
    } else {
        format!("\x1b[{number}~")
    }
}

fn control(key: &str) -> Option<u8> {
    match key {
        "space" | " " | "@" | "2" => Some(0),
        "[" | "3" => Some(0x1b),
        "\\" | "4" => Some(0x1c),
        "]" | "5" => Some(0x1d),
        "^" | "6" => Some(0x1e),
        "_" | "-" | "7" | "/" => Some(0x1f),
        "?" | "8" => Some(0x7f),
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
            Some(key.as_bytes()[0].to_ascii_uppercase() - b'@')
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cursor_and_function_keys_preserve_every_modifier_combination() {
        for application_cursor_keys in [false, true] {
            let modes = Modes {
                application_cursor_keys,
                ..Modes::default()
            };
            for (prefix, modifier) in [
                ("", 1),
                ("shift-", 2),
                ("alt-", 3),
                ("alt-shift-", 4),
                ("ctrl-", 5),
                ("ctrl-shift-", 6),
                ("ctrl-alt-", 7),
                ("ctrl-alt-shift-", 8),
            ] {
                for (key, suffix) in [
                    ("up", 'A'),
                    ("down", 'B'),
                    ("right", 'C'),
                    ("left", 'D'),
                    ("home", 'H'),
                    ("end", 'F'),
                    ("f1", 'P'),
                    ("f4", 'S'),
                ] {
                    let expected = if cfg!(target_os = "macos")
                        && modifier == 3
                        && matches!(key, "left" | "right")
                    {
                        if key == "left" {
                            "\x1bb".into()
                        } else {
                            "\x1bf".into()
                        }
                    } else if modifier != 1 {
                        format!("\x1b[1;{modifier}{suffix}")
                    } else if key.starts_with('f') || application_cursor_keys {
                        format!("\x1bO{suffix}")
                    } else {
                        format!("\x1b[{suffix}")
                    };
                    assert_eq!(
                        encode(
                            &Keystroke::parse(&format!("{prefix}{key}")).unwrap(),
                            modes,
                            true
                        ),
                        Some(expected.into_bytes())
                    );
                }
                for (key, number) in [
                    ("insert", 2),
                    ("delete", 3),
                    ("pageup", 5),
                    ("pagedown", 6),
                    ("f5", 15),
                    ("f12", 24),
                    ("f20", 34),
                ] {
                    let expected = if modifier == 1 {
                        format!("\x1b[{number}~")
                    } else {
                        format!("\x1b[{number};{modifier}~")
                    };
                    assert_eq!(
                        encode(
                            &Keystroke::parse(&format!("{prefix}{key}")).unwrap(),
                            modes,
                            true
                        ),
                        Some(expected.into_bytes())
                    );
                }
            }
        }
    }

    #[test]
    fn option_text_and_control_keys() {
        for (chord, character, expected) in [
            ("alt-b", "∫", "\x1bb"),
            ("alt-shift-b", "ı", "\x1bB"),
            ("alt-.", "≥", "\x1b."),
            ("alt-~", "˘", "\x1b~"),
        ] {
            let mut key = Keystroke::parse(chord).unwrap();
            key.key_char = Some(character.into());
            assert_eq!(
                encode(&key, Modes::default(), true),
                Some(expected.as_bytes().to_vec())
            );
            assert_eq!(
                encode(&key, Modes::default(), false),
                Some(character.as_bytes().to_vec())
            );
            assert!(!uses_text_input(&key, true));
            assert!(uses_text_input(&key, false));
        }
        for (chord, expected) in [
            ("ctrl-a", "\x01"),
            ("ctrl-space", "\0"),
            ("ctrl-backspace", "\x08"),
            ("alt-backspace", "\x1b\x7f"),
            ("ctrl-alt-a", "\x1b\x01"),
            ("alt-space", "\x1b "),
            ("shift-tab", "\x1b[Z"),
        ] {
            for option_as_alt in [false, true] {
                assert_eq!(
                    encode(
                        &Keystroke::parse(chord).unwrap(),
                        Modes::default(),
                        option_as_alt
                    ),
                    Some(expected.as_bytes().to_vec())
                );
            }
        }
        assert_eq!(
            encode(&Keystroke::parse("cmd-c").unwrap(), Modes::default(), true),
            None
        );
        let text = Keystroke {
            key: "a".into(),
            key_char: Some("é".into()),
            modifiers: Modifiers::default(),
        };
        assert!(uses_text_input(&text, true));
    }
}
