use std::collections::{BTreeMap, HashSet};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use muxy_protocol::{
    ErrorCode, ExitReason, HistoryCursor, HistoryPage, SavedScreen, ServerPath, SessionId,
    SessionInfo, Size, TerminalColors, validate_size,
};

use crate::archive::Archive;
use crate::error::ServerError;
use crate::session::{self, SessionHandle};
use crate::settings::ServerSettings;
use crate::spawn::spawn_shell;

type Sessions = Arc<Mutex<State>>;

#[derive(Debug, Default)]
struct State {
    sessions: BTreeMap<SessionId, SessionHandle>,
    starting: HashSet<SessionId>,
    stopping: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerEvent {
    SessionEnded { id: SessionId, reason: ExitReason },
    StopRequested,
    RestartRequested,
}

#[derive(Debug)]
pub struct Registry {
    settings: Mutex<ServerSettings>,
    settings_write: Mutex<()>,
    persist: crate::settings::Persistence,
    shell_integration: Option<crate::ShellIntegration>,
    events: Sender<ServerEvent>,
    sessions: Sessions,
    completed: Arc<Condvar>,
    archive: Archive,
}

impl Registry {
    pub fn new(settings: ServerSettings, events: Sender<ServerEvent>) -> Self {
        Self {
            archive: Archive::memory(settings.history_budget_bytes),
            settings: Mutex::new(settings),
            settings_write: Mutex::new(()),
            persist: crate::settings::Persistence(Box::new(|_| Ok(()))),
            shell_integration: None,
            events,
            sessions: Sessions::default(),
            completed: Arc::default(),
        }
    }

    pub fn persistent(
        settings: ServerSettings,
        events: Sender<ServerEvent>,
        directory: &Path,
    ) -> io::Result<Self> {
        let archive = Archive::open(directory, settings.history_budget_bytes)?;
        Ok(Self {
            archive,
            ..Self::new(settings, events)
        })
    }

    #[must_use]
    pub fn with_shell_integration(mut self, integration: crate::ShellIntegration) -> Self {
        self.shell_integration = Some(integration);
        self
    }

    pub fn settings(&self) -> ServerSettings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn with_settings_persistence(
        mut self,
        persist: impl Fn(&ServerSettings) -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.persist = crate::settings::Persistence(Box::new(persist));
        self
    }

    pub fn write_settings(&self, settings: ServerSettings) -> Result<(), ServerError> {
        settings.validate()?;
        let _write = self
            .settings_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (self.persist.0)(&settings).map_err(|error| {
            ServerError::new(
                ErrorCode::BadRequest,
                format!("Could not save server settings: {error}"),
            )
        })?;
        *self.settings.lock().unwrap_or_else(PoisonError::into_inner) = settings;
        Ok(())
    }

    /// Reserves shutdown against all concurrent session creation.
    pub fn stop_if_idle(&self) -> bool {
        let mut state = lock(&self.sessions);
        if state.stopping || !state.sessions.is_empty() || !state.starting.is_empty() {
            return false;
        }
        state.stopping = true;
        true
    }

    pub fn request_restart(&self) {
        let _ = self.events.send(ServerEvent::RestartRequested);
    }

    pub fn request_stop(&self) {
        let _ = self.events.send(ServerEvent::StopRequested);
    }

    pub fn create(&self, directory: &Path, size: Size) -> Result<SessionInfo, ServerError> {
        self.create_with_colors(directory, size, None)
    }

    pub(crate) fn create_with_colors(
        &self,
        directory: &Path,
        size: Size,
        colors: Option<TerminalColors>,
    ) -> Result<SessionInfo, ServerError> {
        validate_size(size).map_err(|code| {
            ServerError::new(
                code,
                format!("{}x{} is not a valid size", size.cols, size.rows),
            )
        })?;
        let id = loop {
            let id = fresh_id(&self.archive)?;
            let mut state = lock(&self.sessions);
            if state.stopping {
                return Err(ServerError::new(
                    ErrorCode::SpawnFailed,
                    "server is stopping",
                ));
            }
            if !state.sessions.contains_key(&id) && state.starting.insert(id) {
                break id;
            }
        };
        let settings = self.settings();
        let budget = usize::try_from(settings.history_budget_bytes).unwrap_or(usize::MAX);
        let info = SessionInfo {
            id,
            directory: ServerPath(directory.as_os_str().as_bytes().to_vec()),
        };
        let listing = Arc::clone(&self.sessions);
        let events = self.events.clone();
        let completed = Arc::clone(&self.completed);
        let handle = spawn_shell(
            &settings,
            self.shell_integration.as_ref(),
            directory,
            session::pty_size(size),
        )
        .and_then(|pty| {
            session::start(
                info.clone(),
                pty,
                size,
                budget,
                self.archive.clone(),
                colors,
                move |reason| {
                    let mut state = lock(&listing);
                    state.sessions.remove(&id);
                    state.starting.remove(&id);
                    let _ = events.send(ServerEvent::SessionEnded { id, reason });
                    completed.notify_all();
                },
            )
        });
        let mut state = lock(&self.sessions);
        let starting = state.starting.remove(&id);
        self.completed.notify_all();
        let handle = handle?;
        if starting {
            if state.stopping {
                let _ = handle.send(session::SessionCommand::Stop);
            }
            state.sessions.insert(id, handle);
        }
        log::info!("session created: {}", id.get());
        Ok(info)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        lock(&self.sessions)
            .sessions
            .values()
            .map(|handle| handle.info().clone())
            .collect()
    }

    pub fn handle(&self, id: SessionId) -> Option<SessionHandle> {
        lock(&self.sessions).sessions.get(&id).cloned()
    }

    pub fn end(&self, id: SessionId) -> Result<(), ServerError> {
        let handle = self
            .handle(id)
            .ok_or_else(|| ServerError::unknown_session(id))?;
        let _ = handle.send(session::SessionCommand::End);
        self.wait_ended(id)
    }

    pub fn discard(&self, id: SessionId) -> Result<(), ServerError> {
        if let Some(handle) = self.handle(id) {
            let _ = handle.send(session::SessionCommand::End);
            self.wait_ended(id)?;
        }
        self.archive
            .discard(id)
            .map_err(|error| saved_content_error(&error))
    }

    pub fn read_saved_screen(&self, id: SessionId) -> Result<SavedScreen, ServerError> {
        self.archive
            .read(id)
            .map_err(|error| saved_content_error(&error))
    }

    pub fn saved_history_page(
        &self,
        id: SessionId,
        before: HistoryCursor,
        max_rows: u16,
    ) -> Result<HistoryPage, ServerError> {
        self.archive.history_page(id, before, max_rows)
    }

    pub(crate) fn archive(&self) -> Archive {
        self.archive.clone()
    }

    fn wait_ended(&self, id: SessionId) -> Result<(), ServerError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = lock(&self.sessions);
        while state.sessions.contains_key(&id) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ServerError::new(
                    ErrorCode::BadRequest,
                    "session termination timed out",
                ));
            }
            (state, _) = self
                .completed
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner);
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let mut state = lock(&self.sessions);
        if !state.stopping {
            state.stopping = true;
            for handle in state.sessions.values() {
                let _ = handle.send(session::SessionCommand::Stop);
            }
        }
    }

    pub fn is_stopped(&self) -> bool {
        let state = lock(&self.sessions);
        state.stopping && state.sessions.is_empty() && state.starting.is_empty()
    }
}

fn lock(sessions: &Sessions) -> MutexGuard<'_, State> {
    sessions.lock().unwrap_or_else(PoisonError::into_inner)
}

fn fresh_id(archive: &Archive) -> Result<SessionId, ServerError> {
    loop {
        let value = getrandom::u64().map_err(|error| {
            ServerError::new(ErrorCode::SpawnFailed, format!("no randomness: {error}"))
        })?;
        if let Some(id) = SessionId::new(value)
            && !archive.contains(id)
        {
            return Ok(id);
        }
    }
}

fn saved_content_error(error: &io::Error) -> ServerError {
    ServerError::new(
        ErrorCode::SavedContentUnavailable,
        format!("saved terminal content: {error}"),
    )
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn shutdown_reservation_includes_in_progress_spawns() {
        let (events, _receiver) = std::sync::mpsc::channel();
        let registry = Registry::new(ServerSettings::default(), events);
        let session = SessionId::new(1).expect("session");
        lock(&registry.sessions).starting.insert(session);
        assert!(!registry.stop_if_idle());
        assert!(!lock(&registry.sessions).stopping);
        lock(&registry.sessions).starting.remove(&session);
        assert!(registry.stop_if_idle());
        assert!(!registry.stop_if_idle());
    }

    #[test]
    fn creation_racing_idle_shutdown_is_either_preserved_or_rejected() {
        for _ in 0..8 {
            let (events, _receiver) = std::sync::mpsc::channel();
            let registry = Arc::new(Registry::new(
                ServerSettings {
                    default_shell: Some("/bin/sh".into()),
                    ..ServerSettings::default()
                },
                events,
            ));
            let gate = Arc::new(std::sync::Barrier::new(2));
            let spawning = registry.clone();
            let start = gate.clone();
            let worker = std::thread::spawn(move || {
                start.wait();
                spawning.create(Path::new("/tmp"), Size { cols: 80, rows: 24 })
            });
            gate.wait();
            let stopped = registry.stop_if_idle();
            let created = worker.join().expect("spawn thread");
            if stopped {
                assert!(created.is_err());
                assert!(registry.list().is_empty());
            } else {
                let session = created.expect("session preserved");
                assert_eq!(registry.list(), vec![session.clone()]);
                registry.end(session.id).expect("end fixture");
            }
        }
    }
}
