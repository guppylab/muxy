use libproc::net_info::VInfoStat;
use libproc::proc_pid::{self, PIDInfo, PidInfoFlavor};
use libproc::processes::{self, ProcFilter};
use muxy_protocol::ServerPath;

pub(super) fn foreground_member(group: u32) -> Option<(i32, String)> {
    let named = |pid| {
        let pid = i32::try_from(pid).ok()?;
        proc_pid::name(pid).ok().map(|name| (pid, name))
    };
    if let Some(leader) = named(group) {
        return Some(leader);
    }
    let mut members = processes::pids_by_type(ProcFilter::ByProgramGroup { pgrpid: group }).ok()?;
    members.sort_unstable();
    members.into_iter().find_map(named)
}

pub(super) fn process_directory(pid: i32) -> Option<ServerPath> {
    let info = proc_pid::pidinfo::<VnodePathInfo>(pid, 0).ok()?;
    let path = &info.current.path;
    let length = path.iter().position(|byte| *byte == 0)?;
    (path.first() == Some(&b'/')).then(|| ServerPath(path[..length].to_vec()))
}

#[repr(C)]
struct VnodeInfo {
    stat: VInfoStat,
    kind: i32,
    padding: i32,
    filesystem: [i32; 2],
}

#[repr(C)]
struct VnodePath {
    info: VnodeInfo,
    path: [u8; 1024],
}

#[repr(C)]
struct VnodePathInfo {
    current: VnodePath,
    root: VnodePath,
}

impl PIDInfo for VnodePathInfo {
    fn flavor() -> PidInfoFlavor {
        PidInfoFlavor::VNodePathInfo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnode_layout_matches_the_darwin_abi() {
        assert_eq!(size_of::<VnodePathInfo>(), 2352);
        assert_eq!(std::mem::offset_of!(VnodePath, path), 152);
    }
}
