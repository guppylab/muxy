use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use muxy_client::{Client, ClientError};

pub(crate) fn ensure_server_running(socket: &Path) -> Result<Client, ClientError> {
    let _installation = startup_lock(socket)?;
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

fn startup_lock(socket: &Path) -> io::Result<std::fs::File> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match lock_for_update(socket) {
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

pub(crate) fn reconnect_after_update(
    socket: &Path,
    previous_instance: u64,
) -> Result<Client, ClientError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = ensure_server_running(socket);
        match result {
            Ok(client) if client.server_info().instance != previous_instance => return Ok(client),
            Ok(_) if Instant::now() >= deadline => return Err(ClientError::Timeout),
            Ok(_) => {}
            Err(error) if Instant::now() < deadline && retryable_restart(&error) => {}
            Err(error) => return Err(error),
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn retryable_restart(error: &ClientError) -> bool {
    matches!(error, ClientError::Disconnected | ClientError::Timeout)
        || matches!(error, ClientError::Io(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut))
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

pub(crate) fn server_executable() -> io::Result<PathBuf> {
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
    wait_stopped(socket, identity)
}

fn wait_stopped(socket: &Path, identity: (u64, u64)) -> Result<(), ClientError> {
    use std::os::unix::fs::MetadataExt;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpdateMode {
    Preserve,
    WhenIdle,
    EndSessions,
}

pub(crate) fn prepare_update(
    client: &Client,
    socket: &Path,
    update: &crate::updater::PreparedUpdate,
    mode: UpdateMode,
    expected: &muxy_protocol::ServerInfo,
) -> Result<Option<std::fs::File>, ClientError> {
    let installation = match lock_for_update(socket) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    update
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    // A fresh connection protects against a server replaced since the prompt.
    let probe = Client::connect(socket)?;
    if probe.server_info() != expected || client.server_info() != expected {
        return Err(io::Error::other("The server changed. Review the update again.").into());
    }
    match mode {
        UpdateMode::Preserve => {
            if !update.compatible_with(expected) {
                return Err(io::Error::other("This update requires a server restart").into());
            }
        }
        UpdateMode::WhenIdle => {
            if !stop_if_idle(&probe, socket)? {
                return Ok(None);
            }
        }
        UpdateMode::EndSessions => stop_server(&probe, socket)?,
    }
    Ok(Some(installation))
}

fn stop_if_idle(client: &Client, socket: &Path) -> Result<bool, ClientError> {
    use std::os::unix::fs::MetadataExt;
    let identity = std::fs::symlink_metadata(socket).map(|m| (m.dev(), m.ino()))?;
    if !client.stop_server_if_idle()? {
        return Ok(false);
    }
    wait_stopped(socket, identity)?;
    Ok(true)
}

#[derive(Debug)]
pub(crate) struct ServerUpdate {
    pub(crate) server: muxy_protocol::ServerInfo,
    pub(crate) sessions: usize,
    pub(crate) replaced: bool,
}

pub(crate) fn check_update(
    client: &Client,
    socket: &Path,
    replace: bool,
) -> Result<ServerUpdate, ClientError> {
    let server = client.server_info().clone();
    let sessions = client.list_sessions()?.len();
    let mut status = ServerUpdate {
        server,
        sessions,
        replaced: false,
    };
    if replace && sessions == 0 && newer_build(&status.server.build.version) {
        let _installation = match lock_for_update(socket) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(status),
            Err(error) => return Err(error.into()),
        };
        let executable = server_executable()?;
        let installed = read_build_info(&executable)?;
        if installed != muxy_protocol::BuildInfo::current() {
            return Err(io::Error::other(
                "The installed app changed. Reopen Muxy to update the server.",
            )
            .into());
        }
        let probe = Client::connect(socket)?;
        if probe.server_info() == &status.server && stop_if_idle(&probe, socket)? {
            status.replaced = true;
        }
    }
    Ok(status)
}

pub(crate) fn newer_build(running: &str) -> bool {
    match (
        crate::updater::build_number(running),
        crate::updater::build_number(env!("CARGO_PKG_VERSION")),
    ) {
        (Some(running), Some(installed)) => installed > running,
        _ => false,
    }
}

pub(crate) fn read_build_info(executable: &Path) -> io::Result<muxy_protocol::BuildInfo> {
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

pub(crate) fn lock_for_update(socket: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(socket.with_extension("update-lock"))?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "A beta update or server startup is already in progress",
        ),
        std::fs::TryLockError::Error(error) => error,
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
        assert!(
            lock_for_update(&socket).is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
        );
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
