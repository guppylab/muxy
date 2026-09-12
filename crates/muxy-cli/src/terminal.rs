use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::crossterm::{cursor, event, execute, terminal};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGSTOP, SIGTERM, SIGTSTP};

pub(crate) fn require_interactive() -> io::Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        Ok(())
    } else {
        Err(io::Error::other(
            "interactive mode requires terminal stdin and stdout",
        ))
    }
}

pub(crate) fn singleton(profile: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(profile.join("tui.lock"))?;
    file.try_lock()
        .map_err(|_| io::Error::other("another TUI is already running for this profile"))?;
    Ok(file)
}

pub(crate) type Terminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>;

pub(crate) struct Host {
    pub terminal: Terminal,
    pub stop: Arc<AtomicBool>,
    suspend: Arc<AtomicBool>,
    _signals: Signals,
}

impl Host {
    pub(crate) fn enter() -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let suspend = Arc::new(AtomicBool::new(false));
        let mut signals = Signals::default();
        for signal in [SIGHUP, SIGINT, SIGTERM] {
            signals
                .0
                .push(signal_hook::flag::register(signal, Arc::clone(&stop))?);
        }
        signals
            .0
            .push(signal_hook::flag::register(SIGTSTP, Arc::clone(&suspend))?);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
        let mut host = Self {
            terminal: Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?,
            stop,
            suspend,
            _signals: signals,
        };
        host.activate()?;
        Ok(host)
    }

    fn activate(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableBracketedPaste,
            event::EnableFocusChange
        )?;
        self.terminal.clear()
    }

    pub(crate) fn resume_if_needed(&mut self) -> io::Result<bool> {
        if !self.suspend.swap(false, Ordering::AcqRel) {
            return Ok(false);
        }
        restore();
        signal_hook::low_level::raise(SIGSTOP)?;
        self.activate()?;
        Ok(true)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        restore();
    }
}

#[derive(Default)]
struct Signals(Vec<signal_hook::SigId>);

impl Drop for Signals {
    fn drop(&mut self) {
        for signal in self.0.drain(..) {
            signal_hook::low_level::unregister(signal);
        }
    }
}

fn restore() {
    let _ = execute!(
        io::stdout(),
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::SetCursorStyle::DefaultUserShape,
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = io::stdout().flush();
    let _ = terminal::disable_raw_mode();
}
