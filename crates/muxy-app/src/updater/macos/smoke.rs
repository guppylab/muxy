use super::*;
mod tui;
use muxy_client::Client;
use muxy_protocol::Size;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn signed_bundle(path: &Path, server: &Path, identity: &str) -> Result<()> {
    let binaries = path.join("Contents/MacOS");
    std::fs::create_dir_all(&binaries)?;
    std::fs::write(
        path.join("Contents/Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.muxy-beta.app</string>
<key>CFBundleExecutable</key><string>muxy-app</string><key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#,
    )?;
    std::fs::copy("/usr/bin/true", binaries.join("muxy-app"))?;
    std::fs::copy(server, binaries.join("muxy"))?;
    for executable in [binaries.join("muxy-app"), binaries.join("muxy")] {
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--timestamp=none", "--sign", identity])
            .arg(executable))?;
    }
    std::fs::hard_link(binaries.join("muxy"), binaries.join("muxy-server"))?;
    run(Command::new("/usr/bin/codesign")
        .args(["--force", "--timestamp=none", "--sign", identity])
        .arg(path))?;
    Ok(())
}

fn connect(socket: &Path) -> Result<Client> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Client::connect(socket) {
            Ok(client) => return Ok(client),
            Err(error) if Instant::now() >= deadline => return Err(error.into()),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn output(path: &Path) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(value) = std::fs::read_to_string(path)
            && !value.is_empty()
        {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err("Terminal command did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires MUXY_TEST_SERVER, MUXY_TEST_SIGN_IDENTITY and MUXY_TEST_SIGN_TEAM; uses only temporary bundles"]
fn signed_bundle_updates_preserve_shell_and_retire_only_unused_bundles() -> Result<()> {
    let identity = std::env::var("MUXY_TEST_SIGN_IDENTITY")?;
    let team = std::env::var("MUXY_TEST_SIGN_TEAM")?;
    let binary = PathBuf::from(std::env::var("MUXY_TEST_SERVER")?);
    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join("Muxy Beta.app");
    signed_bundle(&bundle, &binary, &identity)?;
    let installation = Installation {
        bundle: bundle.clone(),
        team,
    };
    installation.verify_signature(&bundle, true)?;
    let data = directory.path().join("data");
    std::fs::create_dir(&data)?;
    std::fs::write(data.join("shell-env"), "PS1='lease-test> '\n")?;
    let socket = data.join("server.sock");
    let child = Command::new(bundle.join("Contents/MacOS/muxy"))
        .arg("server")
        .env("MUXY_DIR", &data)
        .env("HOME", &data)
        .env("ENV", data.join("shell-env"))
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut server = Server(child);
    let client = connect(&socket)?;
    let info = client.server_info().clone();
    let size = Size { cols: 80, rows: 24 };
    let session = client.create_session(&data, size)?;
    let attached = client.attach(session.id, size)?;
    client.send_input(attached.channel, b"echo $$ > before.pid\n")?;
    let pid = output(&data.join("before.pid"))?;
    let mut tui = tui::Tui::start(&bundle.join("Contents/MacOS/muxy"), &data)?;
    tui.output("lease-test>")?;
    tui.write(b"printf '\\nLIVE_BUNDLED_TUI\\n'\r")?;
    tui.output("LIVE_BUNDLED_TUI")?;
    let tui_session = client
        .list_sessions()?
        .into_iter()
        .find(|other| other.id != session.id)
        .ok_or("TUI session")?
        .id;
    drop(client);
    let first = replace_bundle(&installation, &info, &binary, &identity)?;
    let client = connect(&socket)?;
    assert_eq!(client.server_info(), &info);
    let attached = client.attach(session.id, size)?;
    client.resize(
        attached.channel,
        Size {
            cols: 100,
            rows: 30,
        },
    )?;
    client.send_input(attached.channel, b"echo $$ > after.pid\n")?;
    assert_eq!(output(&data.join("after.pid"))?, pid);
    assert!(!client.stop_server_if_idle()?);
    let second = replace_bundle(&installation, &info, &binary, &identity)?;
    installation.cleanup(&socket)?;
    assert!(first.join("previous.app").exists());
    assert!(second.join("previous.app").exists());
    client.send_input(attached.channel, b"echo $$ > second.pid\n")?;
    assert_eq!(output(&data.join("second.pid"))?, pid);
    client.end_session(tui_session)?;
    tui.output("Ended")?;
    tui.suspend()?;
    client.end_session(session.id)?;
    assert!(client.stop_server_if_idle()?);
    server.0.wait()?;
    server.0 = Command::new(bundle.join("Contents/MacOS/muxy"))
        .arg("server")
        .env("MUXY_DIR", &data)
        .env("HOME", &data)
        .env("ENV", data.join("shell-env"))
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let replacement = crate::server::reconnect_after_update(&socket, info.instance)?;
    assert_ne!(replacement.server_info().instance, info.instance);
    installation.cleanup(&socket)?;
    assert!(first.join("previous.app").exists());
    assert!(!second.exists());
    tui.resume()?;
    tui.write(b"\x02?")?;
    tui.output("Keyboard help")?;
    tui.write(b"\r\x02d")?;
    tui.exit()?;
    installation.cleanup(&socket)?;
    replacement.stop_server()?;
    server.0.wait()?;
    assert!(!first.exists());
    assert!(!second.exists());
    Ok(())
}

fn replace_bundle(
    installation: &Installation,
    server: &muxy_protocol::ServerInfo,
    binary: &Path,
    identity: &str,
) -> Result<PathBuf> {
    let staging = tempfile::Builder::new()
        .prefix(".muxy-beta-update-")
        .tempdir_in(installation.bundle.parent().ok_or("parent")?)?
        .keep();
    let candidate = staging.join("Muxy Beta.app");
    signed_bundle(&candidate, binary, identity)?;
    installation.verify_signature(&candidate, true)?;
    let update = PreparedUpdate {
        version: env!("CARGO_PKG_VERSION").into(),
        installation: installation.clone(),
        staging: staging.clone(),
        build: Some(crate::server::read_build_info(
            &candidate.join("Contents/MacOS/muxy"),
        )?),
    };
    assert!(update.compatible_with(server));
    update.retain_bundle(server)?;
    replacement::replace(
        &installation.bundle,
        &candidate,
        &staging.join("previous.app"),
    )?;
    installation.verify_signature(&installation.bundle, true)?;
    update.commit_retirement()?;
    Ok(staging)
}

#[test]
#[ignore = "requires MUXY_TEST_LEGACY_SERVER; launches only an isolated temporary server"]
fn beta_mismatch_reports_incompatible_builds_without_stopping_the_old_server() -> Result<()> {
    let executable = std::env::var("MUXY_TEST_LEGACY_SERVER")?;
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("server.sock");
    let child = Command::new(executable)
        .env("MUXY_DIR", directory.path())
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        match Client::connect(&socket) {
            Err(muxy_client::ClientError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => break result,
        }
    };
    assert!(matches!(
        result,
        Err(muxy_client::ClientError::VersionUnsupported)
    ));
    assert!(server.0.try_wait()?.is_none());
    assert!(socket.exists());
    Ok(())
}
