use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientError};

pub(crate) fn ensure_server_running(socket: &Path) -> Result<Client, ClientError> {
    let _installation = lock_for_update(socket)?;
    match Client::connect(socket) {
        Ok(client) => return Ok(client),
        Err(error) if unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    let executable = server_executable()?;
    let mut child = Command::new(&executable)
        .arg("--socket")
        .arg(socket)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not launch {}: {error}; build muxy-server or set MUXY_SERVER_BIN",
                    executable.display()
                ),
            )
        })?;
    thread::spawn(move || {
        let _ = child.wait();
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match connect_before(socket, deadline) {
            Ok(client) => return Ok(client),
            Err(error) if unavailable(&error) => {}
            Err(error) => return Err(error),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "server did not start at {} within 3 seconds",
                    socket.display()
                ),
            )
            .into());
        }
        thread::sleep(remaining.min(Duration::from_millis(25)));
    }
}

fn connect_before(socket: &Path, deadline: Instant) -> Result<Client, ClientError> {
    let socket = socket.to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::Builder::new()
        .name("muxy-server-connect".into())
        .spawn(move || {
            let _ = sender.send(Client::connect(&socket));
        })?;
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => ClientError::Timeout,
            std::sync::mpsc::RecvTimeoutError::Disconnected => ClientError::Disconnected,
        })?
}

fn server_executable() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("MUXY_SERVER_BIN") {
        if path.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_SERVER_BIN must not be empty",
            ));
        }
        return Ok(path.into());
    }
    Ok(std::env::current_exe()?.with_file_name("muxy-server"))
}

fn unavailable(error: &ClientError) -> bool {
    matches!(error, ClientError::Io(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound))
}

pub(crate) fn stop_server(client: &Client, socket: &Path) -> Result<(), ClientError> {
    use std::os::unix::fs::MetadataExt;
    let identity =
        std::fs::symlink_metadata(socket).map(|metadata| (metadata.dev(), metadata.ino()))?;
    client.stop_server()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match std::fs::symlink_metadata(socket) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if (metadata.dev(), metadata.ino()) != identity => return Ok(()),
            Ok(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Server has not finished stopping. Use Connect after shutdown completes.",
            )
            .into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) fn stop_for_update(
    client: &Client,
    socket: &Path,
) -> Result<std::fs::File, ClientError> {
    let installation = lock_for_update(socket)?;
    stop_server(client, socket)?;
    match std::fs::symlink_metadata(socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(installation),
        Err(error) => Err(error.into()),
        Ok(_) => Err(io::Error::other("Another server started during update preparation; close other Muxy Beta instances and retry").into()),
    }
}

fn lock_for_update(socket: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(socket.with_extension("update-lock"))?;
    file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "A beta update or server startup is already in progress: {error}"
        ))
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_startup_respects_the_update_lock_and_releases_it_after_failure() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("server.sock");
        let lock = lock_for_update(&socket)?;
        assert!(
            ensure_server_running(&socket)
                .is_err_and(|error| error.to_string().contains("already in progress"))
        );
        assert!(lock_for_update(&socket).is_err());
        drop(lock);
        assert!(lock_for_update(&socket).is_ok());
        Ok(())
    }

    #[test]
    fn only_a_missing_or_refused_socket_can_start_a_server() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused] {
            assert!(unavailable(&ClientError::Io(kind.into())));
        }
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::TimedOut] {
            assert!(!unavailable(&ClientError::Io(kind.into())));
        }
        assert!(!unavailable(&ClientError::VersionUnsupported));
        assert!(!unavailable(&ClientError::Disconnected));
    }
}
