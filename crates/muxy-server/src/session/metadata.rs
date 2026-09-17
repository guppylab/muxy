#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
use linux::{foreground_member, process_directory};
#[cfg(target_os = "macos")]
use macos::{foreground_member, process_directory};

use muxy_protocol::{ForegroundProcess, MetadataEvent, ServerPath};
use muxy_terminal::TerminalEvent;
use muxy_terminal::pty::Pty;

pub(super) struct Metadata {
    pub(super) title: String,
    pub(super) directory: ServerPath,
    pub(super) process: Option<ForegroundProcess>,
    observed_directory: Option<ServerPath>,
}

impl Metadata {
    pub(super) fn new(directory: ServerPath) -> Self {
        Self {
            title: String::new(),
            directory,
            process: None,
            observed_directory: None,
        }
    }

    pub(super) fn update(&mut self, pty: &Pty, terminal: Vec<TerminalEvent>) -> Vec<MetadataEvent> {
        let mut events = Vec::new();
        let previous_directory = self.directory.clone();
        if let Some(group) = pty.foreground_pid() {
            let member = foreground_member(group);
            let process = ForegroundProcess {
                name: member
                    .as_ref()
                    .map_or_else(String::new, |(_, name)| name.clone()),
                is_shell: group == pty.child_pid(),
            };
            if self.process.as_ref() != Some(&process) {
                events.push(MetadataEvent::ForegroundProcess {
                    name: process.name.clone(),
                    is_shell: process.is_shell,
                });
                self.process = Some(process);
            }
            if let Some(directory) = member.and_then(|(pid, _)| process_directory(pid))
                && self.observed_directory.as_ref() != Some(&directory)
            {
                self.directory.clone_from(&directory);
                self.observed_directory = Some(directory);
            }
        }
        for event in terminal {
            match event {
                TerminalEvent::Title(title) if title != self.title => {
                    self.title.clone_from(&title);
                    events.push(MetadataEvent::Title(title));
                }
                TerminalEvent::Directory(directory) => {
                    if let Some(directory) = terminal_directory(&directory) {
                        self.directory = directory;
                    }
                }
                TerminalEvent::Bell => events.push(MetadataEvent::Bell),
                TerminalEvent::Progress(_) | TerminalEvent::Title(_) => {}
            }
        }
        if self.directory != previous_directory {
            events.push(MetadataEvent::Directory(self.directory.clone()));
        }
        events
    }
}

fn terminal_directory(value: &str) -> Option<ServerPath> {
    if value.starts_with('/') {
        return (!value.contains('\0')).then(|| ServerPath(value.as_bytes().to_vec()));
    }
    let (_, path) = value.strip_prefix("file://")?.split_once('/')?;
    let mut bytes = path.bytes();
    let mut path = vec![b'/'];
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        };
        if byte == 0 {
            return None;
        }
        path.push(byte);
    }
    Some(ServerPath(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn process_directory_preserves_the_kernel_path() -> Result<(), Box<dyn std::error::Error>> {
        let expected = std::env::current_dir()?.canonicalize()?;
        assert_eq!(
            process_directory(i32::try_from(std::process::id())?),
            Some(ServerPath(expected.as_os_str().as_bytes().to_vec()))
        );
        Ok(())
    }

    #[test]
    fn osc_directory_decodes_file_uris_without_losing_path_bytes() {
        for (value, expected) in [
            ("file://localhost/tmp/a%20b", Some(b"/tmp/a b".as_slice())),
            ("file:///tmp/%ff", Some(b"/tmp/\xff".as_slice())),
            ("/tmp/a%20b", Some(b"/tmp/a%20b".as_slice())),
            ("file:///", Some(b"/".as_slice())),
            ("file:///tmp/%00", None),
            ("file:///tmp/%zz", None),
            ("file:///tmp/%a", None),
            ("relative/path", None),
            ("", None),
        ] {
            assert_eq!(
                terminal_directory(value),
                expected.map(|bytes| ServerPath(bytes.to_vec())),
                "{value}"
            );
        }
    }
}
