use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

pub fn muxy_dir() -> io::Result<PathBuf> {
    let linux = cfg!(target_os = "linux");
    let directory = resolve_directory(
        env::var_os("MUXY_DIR"),
        env::var_os("HOME"),
        env::var_os("XDG_STATE_HOME"),
        directory_name(env!("CARGO_PKG_VERSION"), cfg!(debug_assertions), linux),
        linux,
    )?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    Ok(directory)
}

fn directory_name(version: &str, debug: bool, linux: bool) -> &'static str {
    match (debug || version == "2.0.0-beta-0", linux) {
        (true, false) => "Muxy Dev",
        (false, false) => "Muxy Beta",
        (true, true) => "muxy-dev",
        (false, true) => "muxy-beta",
    }
}

fn resolve_directory(
    override_path: Option<OsString>,
    home: Option<OsString>,
    state_home: Option<OsString>,
    name: &str,
    linux: bool,
) -> io::Result<PathBuf> {
    if let Some(path) = override_path {
        return if path.is_empty() {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MUXY_DIR must not be empty",
            ))
        } else {
            Ok(PathBuf::from(path))
        };
    }
    if linux && let Some(state_home) = state_home {
        let state_home = PathBuf::from(state_home);
        if !state_home.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "XDG_STATE_HOME must be absolute",
            ));
        }
        return Ok(state_home.join(name));
    }
    let home = home
        .filter(|home| !home.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join(if linux {
            ".local/state"
        } else {
            "Library/Application Support"
        })
        .join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_builds_do_not_share_beta_data_on_either_platform() -> io::Result<()> {
        for linux in [false, true] {
            for (version, debug, development) in [
                ("2.0.0-beta-0", true, true),
                ("2.0.0-beta-0", false, true),
                ("2.0.0-beta-1001", true, true),
                ("2.0.0-beta-1001", false, false),
            ] {
                let expected = match (linux, development) {
                    (false, true) => "Library/Application Support/Muxy Dev",
                    (false, false) => "Library/Application Support/Muxy Beta",
                    (true, true) => ".local/state/muxy-dev",
                    (true, false) => ".local/state/muxy-beta",
                };
                assert_eq!(
                    resolve_directory(
                        None,
                        Some("/home/test".into()),
                        None,
                        directory_name(version, debug, linux),
                        linux
                    )?,
                    PathBuf::from("/home/test").join(expected)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn explicit_directory_preserves_relative_overrides_and_ignores_invalid_defaults()
    -> io::Result<()> {
        for linux in [false, true] {
            for path in ["/tmp/muxy-test", "relative"] {
                assert_eq!(
                    resolve_directory(
                        Some(path.into()),
                        None,
                        Some("relative-xdg".into()),
                        "name",
                        linux
                    )?,
                    PathBuf::from(path)
                );
            }
            let result = resolve_directory(
                Some(OsString::new()),
                Some("/home/test".into()),
                None,
                "name",
                linux,
            );
            assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
        }
        Ok(())
    }

    #[test]
    fn linux_uses_absolute_xdg_without_requiring_home_and_macos_ignores_it() -> io::Result<()> {
        assert_eq!(
            resolve_directory(None, None, Some("/state".into()), "muxy-beta", true)?,
            PathBuf::from("/state/muxy-beta")
        );
        for state in ["relative", ""] {
            assert!(
                resolve_directory(
                    None,
                    Some("/home/test".into()),
                    Some(state.into()),
                    "muxy-dev",
                    true
                )
                .is_err()
            );
        }
        assert_eq!(
            resolve_directory(
                None,
                Some("/Users/test".into()),
                Some("relative".into()),
                "Muxy Dev",
                false
            )?,
            PathBuf::from("/Users/test/Library/Application Support/Muxy Dev")
        );
        Ok(())
    }

    #[test]
    fn missing_or_empty_home_requires_an_override() {
        for linux in [false, true] {
            for home in [None, Some(OsString::new())] {
                let result = resolve_directory(None, home, None, "name", linux);
                assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::NotFound));
            }
        }
    }
}
