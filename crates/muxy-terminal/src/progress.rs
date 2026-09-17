use std::time::{Duration, Instant};

use crate::TerminalEvent;
use muxy_protocol::{ProgressState, TerminalProgress};

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
    String,
}

#[derive(Debug, Default)]
pub(crate) struct Progress {
    state: State,
    bytes: [u8; 32],
    len: usize,
    utf8_remaining: u8,
    current: Option<TerminalProgress>,
    changed: bool,
    completed: u64,
    deadline: Option<Instant>,
}

impl Progress {
    pub(crate) fn feed(&mut self, mut bytes: &[u8]) {
        while self.utf8_remaining > 0 && !bytes.is_empty() {
            self.advance(bytes[0]);
            bytes = &bytes[1..];
        }
        let text = std::str::from_utf8(bytes).unwrap_or_else(|error| {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        });
        let length = bytes.len();
        while !bytes.is_empty() {
            if matches!(self.state, State::Ground) && self.utf8_remaining == 0 {
                let skip = match text.get(length - bytes.len()..) {
                    Some(text) if !text.is_empty() => text.find('\x1b').unwrap_or(text.len()),
                    _ => bytes
                        .iter()
                        .position(|byte| *byte == 0x1b || !byte.is_ascii())
                        .unwrap_or(bytes.len()),
                };
                bytes = &bytes[skip..];
                if bytes.is_empty() {
                    return;
                }
                if bytes.starts_with(b"\x1b[") {
                    bytes = &bytes[2..];
                    continue;
                }
            }
            self.advance(bytes[0]);
            bytes = &bytes[1..];
        }
    }

    fn advance(&mut self, byte: u8) {
        if self.utf8_remaining > 0 && matches!(byte, 0x80..=0xbf) {
            self.utf8_remaining -= 1;
            self.content(byte);
            return;
        }
        self.utf8_remaining = match byte {
            0xc2..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf4 => 3,
            _ => 0,
        };
        if matches!(byte, 0x18 | 0x1a) {
            self.state = State::Ground;
            return;
        }
        match self.state {
            State::Osc if byte == 0x07 || byte == 0x9c => {
                self.finish();
                self.state = State::Ground;
            }
            State::Osc if byte == 0x1b => self.state = State::OscEscape,
            State::OscEscape if byte == b'\\' => {
                self.finish();
                self.state = State::Ground;
            }
            _ if byte == 0x1b => self.state = State::Escape,
            _ if byte == 0x9d => self.start(),
            _ if matches!(byte, 0x90 | 0x98 | 0x9e | 0x9f) => self.state = State::String,
            _ if matches!(byte, 0x80..=0x8f | 0x91..=0x97 | 0x99..=0x9c) => {
                self.state = State::Ground;
            }
            State::Escape | State::OscEscape => match byte {
                b']' => self.start(),
                b'P' | b'X' | b'^' | b'_' => self.state = State::String,
                b'c' => {
                    self.set(None);
                    self.state = State::Ground;
                }
                0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f => {}
                _ => self.state = State::Ground,
            },
            _ => self.content(byte),
        }
    }

    fn start(&mut self) {
        self.state = State::Osc;
        self.len = 0;
    }

    fn content(&mut self, byte: u8) {
        if !matches!(self.state, State::Osc) || matches!(byte, 0x00..=0x1f | 0x7f) {
            return;
        }
        if let Some(slot) = self.bytes.get_mut(self.len) {
            *slot = byte;
            self.len += 1;
        } else {
            self.len = self.bytes.len() + 1;
        }
    }

    fn finish(&mut self) {
        let Some(bytes) = self.bytes.get(..self.len) else {
            return;
        };
        let Some(report) = bytes.strip_prefix(b"9;4;") else {
            return;
        };
        let mut fields = report.split(|byte| *byte == b';');
        let Some([state @ b'0'..=b'4']) = fields.next() else {
            return;
        };
        let state = *state;
        let percent = match fields.next() {
            None | Some([]) => None,
            Some(digits) if digits.iter().all(u8::is_ascii_digit) => {
                Some(digits.iter().fold(0_u8, |value, digit| {
                    value
                        .saturating_mul(10)
                        .saturating_add(digit - b'0')
                        .min(100)
                }))
            }
            Some(_) => return,
        };
        if fields.next().is_some() {
            return;
        }
        if state == b'0' {
            self.set(None);
            return;
        }
        let previous = self.current.and_then(|progress| progress.percent);
        let (state, percent) = match state {
            b'1' => (ProgressState::Running, Some(percent.unwrap_or(0))),
            b'2' => (ProgressState::Error, percent.or(previous)),
            b'3' => (ProgressState::Indeterminate, previous),
            _ => (ProgressState::Paused, percent.or(previous)),
        };
        self.set(Some(TerminalProgress { state, percent }));
    }

    fn set(&mut self, progress: Option<TerminalProgress>) {
        self.deadline = progress.map(|_| Instant::now() + TIMEOUT);
        if self.current.is_some() && progress.is_none() {
            self.completed = self.completed.saturating_add(1);
        }
        if self.current != progress {
            self.current = progress;
            self.changed = true;
        }
    }

    pub(crate) fn take(&mut self, now: Instant) -> Option<TerminalEvent> {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.set(None);
        }
        std::mem::take(&mut self.changed).then_some(TerminalEvent::Progress(
            muxy_protocol::SessionProgress {
                progress: self.current,
                completed: self.completed,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_reports_refresh_expiry_without_redundant_events()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut progress = Progress::default();
        for byte in b"\x1b]9;4;3\x07" {
            progress.advance(*byte);
        }
        let first = progress.deadline.ok_or("deadline")?;
        assert!(matches!(
            progress.take(Instant::now()),
            Some(TerminalEvent::Progress(muxy_protocol::SessionProgress {
                progress: Some(_),
                ..
            }))
        ));
        progress.deadline = Some(Instant::now());
        for byte in b"\x1b]9;4;3\x07" {
            progress.advance(*byte);
        }
        assert!(progress.deadline >= Some(first));
        let deadline = progress.deadline.ok_or("refreshed deadline")?;
        assert_eq!(
            progress.take(
                deadline
                    .checked_sub(Duration::from_nanos(1))
                    .ok_or("time")?
            ),
            None
        );
        assert_eq!(
            progress.take(deadline),
            Some(TerminalEvent::Progress(muxy_protocol::SessionProgress {
                progress: None,
                completed: 1
            }))
        );
        assert_eq!(progress.take(deadline + TIMEOUT), None);
        Ok(())
    }
}
