use std::ffi::OsString;
use std::fs;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use muxy_client::Client;
use muxy_pty::{ExitStatus, Pty, PtyEvent, PtySize, ReaderHandle, SpawnRequest};
use muxy_terminal::{Size, Terminal};
use serde_json::Value;

pub(super) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const SIZE: Size = Size {
    cols: 100,
    rows: 26,
};

pub(super) struct Fixture {
    pub directory: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn new() -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("muxy-tui-")
            .tempdir_in("/tmp")?;
        fs::create_dir(directory.path().join("home"))?;
        fs::write(
            directory.path().join("server.toml"),
            "default_shell = \"/bin/sh\"\nshell_integration = false\n",
        )?;
        fs::write(directory.path().join("shell-env"), "PS1='tui-test> '\n")?;
        Ok(Self { directory })
    }

    fn environment(&self) -> Vec<(OsString, OsString)> {
        vec![
            ("MUXY_DIR".into(), self.directory.path().as_os_str().into()),
            (
                "HOME".into(),
                self.directory.path().join("home").into_os_string(),
            ),
            (
                "ENV".into(),
                self.directory.path().join("shell-env").into_os_string(),
            ),
            ("TERM".into(), "xterm-256color".into()),
            ("SHELL".into(), "/bin/sh".into()),
        ]
    }

    pub(super) fn command(&self) -> Command {
        let mut command = Command::new(super::support::binary());
        command.envs(self.environment());
        command
    }

    pub(super) fn client(&self) -> Result<Client> {
        Ok(Client::connect(&self.directory.path().join("server.sock"))?)
    }

    pub(super) fn state(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&fs::read(
            self.directory.path().join("tui-state.json"),
        )?)?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(client) = Client::connect_with_timeout(
            &self.directory.path().join("server.sock"),
            Duration::from_millis(200),
        ) {
            let _ = client.stop_server();
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.directory.path().join("server.sock").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

pub(super) struct Tui<'a> {
    fixture: &'a Fixture,
    pub pty: Pty,
    reader: Option<ReaderHandle>,
    events: Receiver<PtyEvent>,
    pub screen: Terminal,
    pub raw: Vec<u8>,
    status: Option<ExitStatus>,
}

impl<'a> Tui<'a> {
    pub(super) fn start(fixture: &'a Fixture, extra: &[(&str, &str)]) -> Result<Self> {
        let mut env = fixture.environment();
        env.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into())),
        );
        let pty = Pty::spawn(SpawnRequest {
            program: super::support::binary(),
            args: Vec::new(),
            cwd: fixture.directory.path().to_owned(),
            env,
            size: PtySize {
                cols: SIZE.cols,
                rows: SIZE.rows,
            },
        })?;
        let (sender, events) = mpsc::channel();
        let reader = pty.start_reader(sender)?;
        Ok(Self {
            fixture,
            pty,
            reader: Some(reader),
            events,
            screen: Terminal::new(SIZE, 1024 * 1024)?,
            raw: Vec::new(),
            status: None,
        })
    }

    pub(super) fn write(&mut self, bytes: &[u8]) -> Result {
        self.pty.write(bytes)?;
        Ok(())
    }

    pub(super) fn pump(&mut self) -> Result {
        if let Ok(PtyEvent::Output(bytes)) = self.events.recv_timeout(Duration::from_millis(20)) {
            self.screen.feed(&bytes);
            self.raw.extend(bytes);
            let response = self.screen.take_pty_output();
            if !response.is_empty() {
                self.pty.write(&response)?;
            }
        }
        Ok(())
    }

    pub(super) fn wait(&mut self, mut predicate: impl FnMut(&mut Self) -> Result<bool>) -> Result {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            self.pump()?;
            if predicate(self)? {
                return Ok(());
            }
            if let Some(status) = self.pty.try_wait() {
                self.status = Some(status);
                break;
            }
        }
        let text = self.text()?;
        Err(format!(
            "TUI wait failed; status={:?}; screen={:?}; state={:?}",
            self.status,
            text,
            self.fixture.state()
        )
        .into())
    }

    pub(super) fn text(&mut self) -> Result<Vec<String>> {
        Ok(self
            .screen
            .screen()?
            .into_iter()
            .map(|row| row.runs.into_iter().map(|run| run.text).collect())
            .collect())
    }

    pub(super) fn output(&mut self, text: &str) -> Result {
        self.wait(|tui| {
            Ok(tui
                .text()?
                .iter()
                .any(|row| row.contains(text) && !row.contains("printf")))
        })
    }

    pub(super) fn ready(&mut self) -> Result {
        self.output("tui-test>")
    }

    pub(super) fn pick(&mut self, picker: u8, index: usize, expected: &str) -> Result {
        self.write(&[0x02, picker])?;
        self.output(expected)?;
        for _ in 0..index {
            self.write(b"\x1b[B")?;
        }
        self.write(b"\r")
    }

    pub(super) fn tabs(&self) -> Result<Vec<Value>> {
        let state = self.fixture.state()?;
        let active = state["active"].as_str().ok_or("active project")?;
        Ok(state["projects"][active]["tabs"]
            .as_array()
            .ok_or("tabs")?
            .clone())
    }

    pub(super) fn active_tab(&self) -> Result<Value> {
        let state = self.fixture.state()?;
        let active = state["active"].as_str().ok_or("active project")?;
        let project = &state["projects"][active];
        let index = project["active"].as_u64().ok_or("active tab")?;
        Ok(project["tabs"][usize::try_from(index)?].clone())
    }

    pub(super) fn exit(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            self.pump()?;
            if let Some(status) = self.pty.try_wait() {
                self.status = Some(status);
                self.pump()?;
                return Ok(status);
            }
        }
        Err("TUI did not exit".into())
    }

    pub(super) fn detach(&mut self) -> Result {
        self.write(b"\x02d")?;
        assert_eq!(self.exit()?.code, Some(0));
        assert!(self.raw.windows(8).any(|bytes| bytes == b"\x1b[?1049l"));
        Ok(())
    }
}

impl Drop for Tui<'_> {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.pty.kill();
            let _ = self.pty.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
