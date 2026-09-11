use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;

pub fn muxy_dir() -> io::Result<PathBuf> {
    let directory = resolve_directory(
        env::var_os("MUXY_DIR"),
        env::var_os("HOME"),
        directory_name(env!("CARGO_PKG_VERSION"), cfg!(debug_assertions)),
    )?;
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn directory_name(version: &str, debug: bool) -> &'static str {
    if debug || version == "2.0.0-beta-0" {
        "Muxy Dev"
    } else {
        "Muxy Beta"
    }
}

fn resolve_directory(
    override_path: Option<OsString>,
    home: Option<OsString>,
    name: &str,
) -> io::Result<PathBuf> {
    let directory = match override_path {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_DIR must not be empty",
            ));
        }
        None => home
            .filter(|home| !home.is_empty())
            .map(|home| {
                PathBuf::from(home)
                    .join("Library/Application Support")
                    .join(name)
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?,
    };
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_builds_do_not_share_beta_data() -> io::Result<()> {
        let home = Some(OsString::from("/Users/test"));
        for (version, debug, expected) in [
            ("2.0.0-beta-0", true, "Muxy Dev"),
            ("2.0.0-beta-0", false, "Muxy Dev"),
            ("2.0.0-beta-1001", true, "Muxy Dev"),
            ("2.0.0-beta-1001", false, "Muxy Beta"),
        ] {
            let directory = resolve_directory(None, home.clone(), directory_name(version, debug))?;
            assert_eq!(
                directory,
                PathBuf::from("/Users/test/Library/Application Support").join(expected)
            );
        }
        Ok(())
    }

    #[test]
    fn explicit_directory_overrides_both_defaults_without_home() -> io::Result<()> {
        for name in ["Muxy Dev", "Muxy Beta"] {
            let directory = resolve_directory(Some(OsString::from("/tmp/muxy-test")), None, name)?;
            assert_eq!(directory, PathBuf::from("/tmp/muxy-test"));
        }
        Ok(())
    }

    #[test]
    fn empty_override_is_rejected_instead_of_falling_back() {
        let result = resolve_directory(
            Some(OsString::new()),
            Some(OsString::from("/Users/test")),
            "Muxy Dev",
        );
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    }

    #[test]
    fn missing_or_empty_home_requires_an_override() {
        for home in [None, Some(OsString::new())] {
            let result = resolve_directory(None, home, "Muxy Dev");
            assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::NotFound));
        }
    }
}
