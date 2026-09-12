use std::path::PathBuf;

pub(crate) fn binary() -> PathBuf {
    std::env::var_os("MUXY_TEST_RUNTIME")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_muxy")), PathBuf::from)
}
