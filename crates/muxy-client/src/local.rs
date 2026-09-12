pub mod bundle;

use crate::{Client, ClientError};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn server_executable() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("MUXY_SERVER_BIN") {
        if path.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_SERVER_BIN must not be empty",
            ));
        }
        return Ok(path.into());
    }
    Ok(std::env::current_exe()?
        .canonicalize()?
        .with_file_name("muxy-server"))
}

pub fn ensure_running(socket: &Path, executable: &Path) -> Result<Client, ClientError> {
    let _startup = wait_for_startup_lock(socket)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    match Client::connect_with_timeout(socket, deadline.saturating_duration_since(Instant::now())) {
        Ok(client) => return Ok(client),
        Err(error) if unavailable(&error) => {}
        Err(error) => return Err(error),
    }
    let mut child = Command::new(executable)
        .arg("--socket")
        .arg(socket)
        .arg("--settings")
        .arg(socket.with_file_name("server.toml"))
        .arg("--log")
        .arg(socket.with_file_name("server.log"))
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not launch server {}: {error}", executable.display()),
            )
        })?;
    thread::Builder::new()
        .name("muxy-server-wait".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ClientError::Timeout);
        }
        match Client::connect_with_timeout(socket, remaining) {
            Ok(client) => return Ok(client),
            Err(error) if unavailable(&error) => {}
            Err(error) => return Err(error),
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        );
    }
}

pub fn unavailable(error: &ClientError) -> bool {
    matches!(error, ClientError::Io(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound))
}

fn wait_for_startup_lock(socket: &Path) -> io::Result<File> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match lock_startup(socket) {
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

pub fn lock_startup(socket: &Path) -> io::Result<File> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    try_lock(&socket.with_extension("update-lock"))
}

pub fn try_lock(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "An installation or server startup is already in progress",
        ),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

pub fn read_build_info(executable: &Path) -> io::Result<muxy_protocol::BuildInfo> {
    use std::io::{Read, Seek};
    let mut output = tempfile::tempfile()?;
    let mut child = Command::new(executable)
        .arg("--build-info")
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(io::Error::other("Server build metadata is unavailable"));
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Server build metadata timed out",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    output.rewind()?;
    let mut bytes = Vec::new();
    output.take(4097).read_to_end(&mut bytes)?;
    let info: muxy_protocol::BuildInfo = serde_json::from_slice(&bytes)?;
    if bytes.len() > 4096 || info.version.len() > 128 || info.compatibility == 0 {
        return Err(io::Error::other("Invalid server build metadata"));
    }
    Ok(info)
}
