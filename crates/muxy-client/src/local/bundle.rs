use std::io;
use std::path::Path;

pub use muxy_core::bundle::{BundleLease, lock_replacement, lock_unused};

pub fn acquire_runtime(executable: &Path) -> io::Result<Option<BundleLease>> {
    muxy_core::bundle::acquire_runtime(
        executable,
        &serde_json::to_vec(&muxy_protocol::BuildInfo::current())?,
    )
}
