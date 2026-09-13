use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

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
