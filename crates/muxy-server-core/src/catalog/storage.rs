use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::State;

pub(super) fn save(path: &Path, state: &State, committed: &mut bool) -> io::Result<()> {
    let bytes = serde_json::to_vec(state)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("catalog has no parent"))?;
    let temporary = parent.join(format!(
        ".catalog-{}.tmp",
        muxy_protocol::OperationId::new()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        *committed = true;
        sync_parent(path)
    })();
    if !*committed {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn sync_parent(path: &Path) -> io::Result<()> {
    let directory = File::open(
        path.parent()
            .ok_or_else(|| io::Error::other("catalog has no parent"))?,
    )?;
    if !directory.metadata()?.is_dir() {
        return Err(io::Error::other("catalog parent is not a directory"));
    }
    directory.sync_all()
}
