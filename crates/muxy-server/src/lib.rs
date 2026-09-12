//! Server runtime composition and lifecycle, shared by command-line entry points.

mod args;
mod logging;
mod run;
mod settings_file;

mod legacy;

pub fn run(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> std::io::Result<()> {
    args::Args::parse(arguments).and_then(|args| run::run(&args))
}
