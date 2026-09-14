use super::{Result, command::capture, error};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub(super) fn stop(directory: &Path) -> Result<()> {
    let directory = directory.canonicalize().map_err(error)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let processes: Vec<_> = directories()?
            .into_iter()
            .filter(|(_, cwd)| cwd.starts_with(&directory))
            .collect();
        if processes.is_empty() {
            return Ok(());
        }
        if processes
            .iter()
            .any(|(pid, _)| *pid <= 1 || *pid == std::process::id())
        {
            return Err(error(
                "The server is running from this worktree; files were preserved",
            ));
        }
        if Instant::now() >= deadline {
            return Err(error(
                "Processes are still using this worktree; files were preserved",
            ));
        }
        for (pid, _) in processes {
            let Some(before) = identity(pid) else {
                continue;
            };
            for signal in ["-TERM", "-KILL"] {
                let still_here = directories()?
                    .iter()
                    .any(|(current, cwd)| *current == pid && cwd.starts_with(&directory));
                if !still_here || identity(pid).as_ref() != Some(&before) {
                    break;
                }
                let mut command = Command::new("/bin/kill");
                command.args([signal, "--", &pid.to_string()]);
                if let Err(e) = capture(command)
                    && identity(pid).as_ref() == Some(&before)
                {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn directories() -> Result<Vec<(u32, PathBuf)>> {
    use std::os::unix::ffi::OsStrExt;
    let mut command = Command::new("/usr/sbin/lsof");
    command.args(["-n", "-P", "-a", "-d", "cwd", "-Fpn0"]);
    let bytes = capture(command)?;
    let mut pid = None;
    let mut result = Vec::new();
    for record in bytes.split(|b| *b == 0) {
        let record = record.strip_prefix(b"\n").unwrap_or(record);
        if let Some(value) = record.strip_prefix(b"p") {
            pid = std::str::from_utf8(value).ok().and_then(|s| s.parse().ok());
        }
        if let Some(value) = record.strip_prefix(b"n")
            && let Some(pid) = pid
        {
            result.push((pid, PathBuf::from(std::ffi::OsStr::from_bytes(value))));
        }
    }
    Ok(result)
}
#[cfg(target_os = "macos")]
fn identity(pid: u32) -> Option<String> {
    use libproc::libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};
    let info = pidinfo::<BSDInfo>(i32::try_from(pid).ok()?, 0).ok()?;
    Some(format!(
        "{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}
#[cfg(target_os = "linux")]
fn directories() -> Result<Vec<(u32, PathBuf)>> {
    Ok(std::fs::read_dir("/proc")
        .map_err(error)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse().ok()?;
            Some((pid, std::fs::read_link(entry.path().join("cwd")).ok()?))
        })
        .collect())
}
#[cfg(target_os = "linux")]
fn identity(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
