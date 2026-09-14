use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Result, error};

const LIMIT: usize = 4 * 1024 * 1024;

pub(super) fn run(path: &Path, args: &[impl AsRef<OsStr>]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .arg("--no-pager")
        .arg("--literal-pathspecs")
        .args(args)
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .process_group(0);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    capture(command)
}

pub(super) fn capture(mut command: Command) -> Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(error)?;
    let id = child.id();
    let (send, receive) = mpsc::sync_channel(16);
    for (stream, pipe) in [
        (
            0,
            Box::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| error("missing Git stdout"))?,
            ) as Box<dyn Read + Send>,
        ),
        (
            1,
            Box::new(
                child
                    .stderr
                    .take()
                    .ok_or_else(|| error("missing Git stderr"))?,
            ) as Box<dyn Read + Send>,
        ),
    ] {
        let send = send.clone();
        std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if send.send((stream, Ok(buffer[..n].to_vec()))).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = send.send((stream, Err(e)));
                        break;
                    }
                }
            }
        });
    }
    drop(send);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut output = [Vec::new(), Vec::new()];
    let result = (|| {
        loop {
            if Instant::now() >= deadline {
                return Err(error("Git command timed out; refresh before retrying"));
            }
            match receive.recv_timeout(Duration::from_millis(10)) {
                Ok((stream, bytes)) => {
                    let bytes = bytes.map_err(error)?;
                    if output[stream].len() + bytes.len() > LIMIT {
                        return Err(error("Git output exceeds the limit"));
                    }
                    output[stream].extend(bytes);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(status) = child.try_wait().map_err(error)? {
                        return if status.success() {
                            Ok(std::mem::take(&mut output[0]))
                        } else {
                            Err(error(String::from_utf8_lossy(&output[1]).trim()))
                        };
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    })();
    if result.is_err() {
        terminate_group(id);
    }
    let _ = child.wait();
    result
}

fn terminate_group(id: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{id}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
