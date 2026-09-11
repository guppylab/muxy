//! Server executable composition root and process lifecycle.

mod args;
mod logging;
mod run;
mod settings_file;

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("--build-info")])
    {
        return match serde_json::to_writer(
            io::stdout().lock(),
            &muxy_protocol::BuildInfo::current(),
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    match args::Args::parse(std::env::args_os().skip(1)).and_then(|args| run::run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("{error}");
            let _ = writeln!(io::stderr(), "muxy-server: {error}");
            ExitCode::FAILURE
        }
    }
}
