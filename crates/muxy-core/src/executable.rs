use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn current_path() -> io::Result<std::path::PathBuf> {
    // macOS current_exe can retain the launch symlink after its target changes.
    #[cfg(target_os = "macos")]
    let path = std::path::PathBuf::from(
        libproc::libproc::proc_pid::pidpath(
            i32::try_from(std::process::id()).map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?,
    );
    #[cfg(not(target_os = "macos"))]
    let path = std::env::current_exe()?;
    path.canonicalize()
}

pub fn build_metadata(executable: &Path) -> io::Result<Vec<u8>> {
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
    if bytes.len() > 4096 {
        return Err(io::Error::other("Executable build metadata is too large"));
    }
    Ok(bytes)
}
