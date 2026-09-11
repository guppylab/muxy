use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Result, build_number, client, download, latest, replacement};

#[derive(Clone)]
pub(crate) struct Installation {
    bundle: PathBuf,
    team: String,
}

pub(crate) struct PreparedUpdate {
    pub(crate) version: String,
    installation: Installation,
    staging: tempfile::TempDir,
}

impl Installation {
    pub(crate) fn detect() -> Result<Self> {
        build_number(env!("CARGO_PKG_VERSION"))
            .ok_or("Automatic updates require an installed release of Muxy Beta")?;
        let executable = std::env::current_exe()?.canonicalize()?;
        let bundle = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or("Automatic updates require an installed Muxy Beta.app")?
            .to_owned();
        if bundle.starts_with("/Volumes")
            || bundle
                .components()
                .any(|part| part.as_os_str() == "AppTranslocation")
        {
            return Err("Move Muxy Beta.app to Applications and reopen it before updating".into());
        }
        let details = Command::new("/usr/bin/codesign")
            .args(["--display", "--verbose=4"])
            .arg(&bundle)
            .output()?;
        if !details.status.success() {
            return Err("The installed beta has no valid developer signature".into());
        }
        let details = String::from_utf8(details.stderr)?;
        let team = details
            .lines()
            .find_map(|line| line.strip_prefix("TeamIdentifier="))
            .filter(|team| {
                team.len() == 10
                    && team
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            })
            .ok_or("Automatic updates require a Developer ID signed beta")?
            .to_owned();
        let installation = Self { bundle, team };
        installation.verify_app(&installation.bundle, env!("CARGO_PKG_VERSION"))?;
        Ok(installation)
    }

    pub(crate) fn prepare(&self) -> Result<Option<PreparedUpdate>> {
        let client = client()?;
        let (platform, arch) = if cfg!(target_arch = "aarch64") {
            ("macos-aarch64", "arm64")
        } else {
            ("macos-x86_64", "x86_64")
        };
        let Some(release) = latest(&client, platform, arch)? else {
            return Ok(None);
        };
        let staging = tempfile::Builder::new()
            .prefix(".muxy-beta-update-")
            .tempdir_in(
                self.bundle
                    .parent()
                    .ok_or("Missing application directory")?,
            )?;
        let download_dir = tempfile::tempdir()?;
        let dmg = download_dir.path().join("update.dmg");
        download(&client, &release, &dmg)?;
        self.verify_signature(&dmg, false)?;
        let mount = MountedImage::attach(&dmg, download_dir.path())?;
        let source = mount.path.join("Muxy Beta.app");
        self.verify_app(&source, &release.version)?;
        let candidate = staging.path().join("Muxy Beta.app");
        run(Command::new("/usr/bin/ditto").arg(&source).arg(&candidate))?;
        self.verify_app(&candidate, &release.version)?;
        Ok(Some(PreparedUpdate {
            version: release.version,
            installation: self.clone(),
            staging,
        }))
    }

    fn verify_signature(&self, path: &Path, app: bool) -> Result<()> {
        let mut requirement = format!(
            "anchor apple generic and certificate leaf[subject.OU] = \"{}\"",
            self.team
        );
        if app {
            requirement.push_str(" and identifier \"com.muxy-beta.app\"");
        }
        verify_code(path, &requirement)
    }

    fn verify_app(&self, app: &Path, version: &str) -> Result<()> {
        self.verify_signature(app, true)?;
        let plist = app.join("Contents/Info.plist");
        for (key, expected) in [
            ("CFBundleIdentifier", "com.muxy-beta.app"),
            ("CFBundleExecutable", "muxy-app"),
            ("MuxyVersion", version),
        ] {
            let actual = run(Command::new("/usr/bin/plutil")
                .args(["-extract", key, "raw", "-o", "-"])
                .arg(&plist))?;
            if actual.trim() != expected {
                return Err(format!("The signed beta has an unexpected {key}").into());
            }
        }
        let count = run(Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleVersion", "raw", "-o", "-"])
            .arg(&plist))?;
        if count.trim().parse::<u64>().ok() != build_number(version) {
            return Err("The signed beta build number does not match the update feed".into());
        }
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        for binary in ["muxy-app", "muxy-server"] {
            run(Command::new("/usr/bin/lipo")
                .arg(app.join("Contents/MacOS").join(binary))
                .args(["-verify_arch", arch]))?;
        }
        Ok(())
    }
}

impl PreparedUpdate {
    #[cfg(test)]
    pub(crate) fn fixture() -> Result<Self> {
        let staging = tempfile::tempdir()?;
        Ok(Self {
            version: "2.0.0-beta-1234".into(),
            installation: Installation {
                bundle: staging.path().join("installed.app"),
                team: "TESTTEAM00".into(),
            },
            staging,
        })
    }

    pub(crate) fn install(self, _server_lock: std::fs::File) -> Result<tempfile::TempDir> {
        let _bundle_lock = replacement::lock(
            self.installation
                .bundle
                .parent()
                .ok_or("Missing application directory")?,
        )?;
        let candidate = self.staging.path().join("Muxy Beta.app");
        self.installation.verify_app(&candidate, &self.version)?;
        self.installation
            .verify_app(&self.installation.bundle, env!("CARGO_PKG_VERSION"))?;
        let backup = self.staging.path().join("previous.app");
        if let Err(error) =
            replacement::replace_and_restart(&self.installation.bundle, &candidate, &backup, || {
                restart(&self.installation.bundle)
            })
        {
            if backup.exists() {
                let _ = self.staging.keep();
                return Err(format!(
                    "{error}. The previous app is available at {}",
                    backup.display()
                )
                .into());
            }
            return Err(error);
        }
        Ok(self.staging)
    }
}

fn restart(bundle: &Path) -> Result<()> {
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.1; done; /usr/bin/open \"$2\"",
            "muxy-beta-restart",
        ])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

struct MountedImage {
    path: PathBuf,
}

impl MountedImage {
    fn attach(dmg: &Path, directory: &Path) -> Result<Self> {
        let path = directory.join("mount");
        std::fs::create_dir(&path)?;
        run(Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&path)
            .arg(dmg)
            .stdin(std::process::Stdio::null()))?;
        Ok(Self { path })
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/hdiutil")
            .args(["detach", "-force"])
            .arg(&self.path)
            .output();
    }
}

fn run(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            command.get_program().to_string_lossy(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn verify_code(path: &Path, requirement: &str) -> Result<()> {
    run(Command::new("/usr/bin/codesign")
        .args([
            "--verify",
            "--strict",
            "--deep",
            "-R",
            &format!("={requirement}"),
        ])
        .arg(path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_verification_checks_literal_requirement_and_nested_code() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let app = directory.path().join("Muxy Beta.app");
        let binaries = app.join("Contents/MacOS");
        std::fs::create_dir_all(&binaries)?;
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.muxy-beta.app</string>
<key>CFBundleExecutable</key><string>muxy-app</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#,
        )?;
        for binary in ["muxy-app", "muxy-server"] {
            let path = binaries.join(binary);
            std::fs::copy("/usr/bin/true", &path)?;
            run(Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&path))?;
        }
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app))?;
        verify_code(&app, "identifier \"com.muxy-beta.app\"")?;
        assert!(verify_code(&app, "identifier \"com.muxy.app\"").is_err());
        let installation = Installation {
            bundle: app.clone(),
            team: "TESTTEAM00".into(),
        };
        assert!(installation.verify_signature(&app, true).is_err());
        std::fs::write(binaries.join("muxy-server"), b"changed helper")?;
        assert!(verify_code(&app, "identifier \"com.muxy-beta.app\"").is_err());
        Ok(())
    }
}
