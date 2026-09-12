use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Help,
    Version,
    BuildInfo,
    Interactive,
    Server(Vec<OsString>),
    Projects,
    AddProject {
        directory: PathBuf,
        name: Option<String>,
    },
}

pub(crate) fn parse(executable: &Path, arguments: Vec<OsString>) -> io::Result<Command> {
    match arguments.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => return Ok(Command::Help),
        [flag] if flag == "--version" || flag == "-V" => return Ok(Command::Version),
        [flag] if flag == "--build-info" => return Ok(Command::BuildInfo),
        _ => {}
    }
    if executable
        .file_name()
        .is_some_and(|name| name == "muxy-server")
        && arguments
            .first()
            .is_none_or(|argument| argument != "server")
    {
        return Ok(Command::Server(arguments));
    }
    match arguments.as_slice() {
        [] => Ok(Command::Interactive),
        [command, rest @ ..] if command == "server" => Ok(Command::Server(rest.to_vec())),
        [command, action] if command == "project" && action == "list" => Ok(Command::Projects),
        [command, action, directory] if command == "project" && action == "add" => {
            Ok(Command::AddProject {
                directory: directory.into(),
                name: None,
            })
        }
        [command, action, directory, flag, name]
            if command == "project" && action == "add" && flag == "--name" =>
        {
            let name = name
                .to_str()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| invalid("project name must be nonempty UTF-8"))?;
            Ok(Command::AddProject {
                directory: directory.into(),
                name: Some(name.trim().into()),
            })
        }
        _ => Err(invalid("unknown command or arguments; run muxy --help")),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
