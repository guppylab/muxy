mod io;
pub(crate) use io::{InputWriter, Shared, View, lock};

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use muxy_app_core::{Direction, PaneId};
use muxy_client::{Client, ClientError, RunGrid};
use muxy_protocol::{
    CatalogPage, ErrorCode, ProjectId, ProjectSession, ServerIdentity, SessionId, Size,
};
use ratatui::layout::Rect;

use crate::state::{Discard, Result, Store};

#[derive(Clone, Debug)]
pub(crate) enum Action {
    Open(ProjectId),
    New(Option<Direction>),
    SelectTab(usize),
    CycleTab(bool),
    Focus(Direction),
    Resize(Direction),
    Zoom,
    Close(PaneId),
    Existing(ProjectSession),
    ListSessions,
    Detach,
}

pub(crate) struct Worker {
    pub shared: Arc<Mutex<Shared>>,
    pub input: InputWriter,
    pub viewport: Arc<Mutex<Rect>>,
    sender: SyncSender<Action>,
    stop: Arc<AtomicBool>,
    connection: Arc<Mutex<Option<Client>>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn start(profile: PathBuf, executable: PathBuf, viewport: Rect) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let input = InputWriter::new(Arc::clone(&shared))?;
        let viewport = Arc::new(Mutex::new(viewport));
        let stop = Arc::new(AtomicBool::new(false));
        let connection = Arc::new(Mutex::new(None));
        let (sender, receiver) = mpsc::sync_channel(32);
        let mut core = Core {
            profile,
            executable,
            shared: Arc::clone(&shared),
            viewport: Arc::clone(&viewport),
            stop: Arc::clone(&stop),
            connection: Arc::clone(&connection),
            store: None,
        };
        let worker = thread::Builder::new()
            .name("muxy-tui-requests".into())
            .spawn(move || {
                core.run(&receiver);
                lock(&core.shared).done = true;
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            shared,
            input,
            viewport,
            sender,
            stop,
            connection,
            thread: Some(worker),
        })
    }

    pub(crate) fn send(&self, action: Action) -> Result {
        self.sender
            .try_send(action)
            .map_err(|_| "TUI is busy; wait for the pending action".into())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(client) = lock(&self.connection).take() {
            client.disconnect();
        }
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

struct Core {
    profile: PathBuf,
    executable: PathBuf,
    shared: Arc<Mutex<Shared>>,
    viewport: Arc<Mutex<Rect>>,
    stop: Arc<AtomicBool>,
    connection: Arc<Mutex<Option<Client>>>,
    store: Option<Store>,
}

impl Core {
    fn run(&mut self, requests: &Receiver<Action>) {
        while !self.stop.load(Ordering::Acquire) {
            self.message("Connecting to local server…");
            let client = match muxy_client::local::ensure_running(
                &self.profile.join("server.sock"),
                &self.executable,
            ) {
                Ok(client) => client,
                Err(error) => {
                    self.message(&error.to_string());
                    if matches!(
                        requests.recv_timeout(Duration::from_millis(500)),
                        Ok(Action::Detach) | Err(mpsc::RecvTimeoutError::Disconnected)
                    ) {
                        break;
                    }
                    continue;
                }
            };
            *lock(&self.connection) = Some(client.clone());
            let reader = match io::reader(client.clone(), Arc::clone(&self.shared)) {
                Ok(reader) => reader,
                Err(error) => {
                    self.message(&error);
                    break;
                }
            };
            let result = self.connected(&client, requests);
            client.disconnect();
            let _ = reader.join();
            lock(&self.shared).disconnected();
            *lock(&self.connection) = None;
            match result {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    let mut shared = lock(&self.shared);
                    shared.message = error;
                    shared.failed = true;
                    break;
                }
            }
        }
    }

    fn connected(&mut self, client: &Client, requests: &Receiver<Action>) -> Result<bool> {
        let mut catalog = client.catalog().map_err(|error| error.to_string())?;
        if self.store.is_none() {
            self.store = Some(Store::load(&self.profile, &catalog)?);
        }
        self.store_mut()?
            .change(|state| state.reconcile(&catalog))?;
        self.publish(&catalog);
        lock(&self.shared).client = Some(client.clone());
        self.message("");
        while !self.stop.load(Ordering::Acquire) && client.is_connected() {
            let refresh = {
                let mut shared = lock(&self.shared);
                std::mem::take(&mut shared.refresh)
            };
            if refresh {
                match client.catalog() {
                    Ok(next) => {
                        self.store_mut()?.change(|state| state.reconcile(&next))?;
                        catalog = next;
                        self.publish(&catalog);
                    }
                    Err(error) => self.message(&error.to_string()),
                }
            }
            if let Err(error) = self.synchronize(client, &catalog) {
                self.message(&error);
            }
            match requests.recv_timeout(Duration::from_millis(100)) {
                Ok(Action::Detach) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(true),
                Ok(action) => {
                    if let Err(error) = self.action(action, client, &catalog) {
                        self.message(&error);
                    }
                    self.publish(&catalog);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(self.stop.load(Ordering::Acquire))
    }

    fn action(&mut self, action: Action, client: &Client, catalog: &CatalogPage) -> Result {
        if matches!(action, Action::ListSessions) {
            return self.list_sessions(client, catalog);
        }
        let host = hosting_session(catalog.server);
        self.store_mut()?.change(|state| {
            match action {
                Action::Open(id) => state.open(id, catalog)?,
                Action::New(split) => {
                    let directory = catalog
                        .projects
                        .iter()
                        .find(|project| project.id == state.active)
                        .ok_or("Project is no longer available")?
                        .directory
                        .clone();
                    state.new_pane(split, directory, None)?;
                }
                Action::Existing(session) => {
                    if Some(session.info.id) == host {
                        return Err("Cannot attach the terminal hosting this TUI".into());
                    }
                    if session.info.project != state.active {
                        return Err("Terminal belongs to another project".into());
                    }
                    state.new_pane(None, session.info.directory, Some(session.info.id))?;
                }
                Action::Close(id) => {
                    if state
                        .tab()
                        .and_then(|tab| tab.panes.get(&id))
                        .is_some_and(|pane| pane.session.is_some() && pane.session == host)
                    {
                        return Err("Cannot close the terminal hosting this TUI".into());
                    }
                    state.close(id)?;
                }
                Action::Focus(direction) => state.focus(direction),
                Action::Resize(direction) => state.resize(direction)?,
                Action::Zoom => {
                    if let Some(tab) = state.tab_mut() {
                        tab.zoom = !tab.zoom;
                    }
                }
                Action::SelectTab(index) => {
                    if let Some(project) = state.projects.get_mut(&state.active)
                        && index < project.tabs.len()
                    {
                        project.active = index;
                    }
                }
                Action::CycleTab(forward) => {
                    if let Some(project) = state.projects.get_mut(&state.active)
                        && !project.tabs.is_empty()
                    {
                        project.active = (project.active
                            + if forward { 1 } else { project.tabs.len() - 1 })
                            % project.tabs.len();
                    }
                }
                Action::ListSessions | Action::Detach => {}
            }
            Ok(())
        })?;
        self.message("");
        Ok(())
    }

    fn synchronize(&mut self, client: &Client, catalog: &CatalogPage) -> Result {
        self.store_mut()?.ready()?;
        while let Some(discard) = self.store_mut()?.state.discards.first().copied() {
            match discard {
                Discard::Session(session) => client.discard_session(session),
                Discard::Creation(operation) => client.cancel_creation(operation),
            }
            .map_err(|error| error.to_string())?;
            self.store_mut()?.change(|state| {
                state.discards.remove(0);
                Ok(())
            })?;
        }
        let viewport = *lock(&self.viewport);
        let state = self.store_mut()?.state.clone();
        let regions = crate::render::regions(&state, viewport);
        let desired: BTreeMap<_, _> = regions
            .iter()
            .filter_map(|(id, rect)| {
                let size = crate::render::terminal_size(*rect)?;
                Some((*id, size))
            })
            .collect();
        let obsolete: Vec<_> = lock(&self.shared)
            .views
            .iter()
            .filter(|(id, _)| !desired.contains_key(id))
            .map(|(id, view)| (*id, view.channel))
            .collect();
        for (id, channel) in obsolete {
            if let Some(channel) = channel {
                match client.detach(channel) {
                    Ok(()) => {}
                    Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownChannel => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            lock(&self.shared).views.remove(&id);
        }
        for (id, size) in desired {
            if self.stop.load(Ordering::Acquire) {
                break;
            }
            self.synchronize_pane(client, catalog, id, size)?;
        }
        self.publish(catalog);
        Ok(())
    }

    fn synchronize_pane(
        &mut self,
        client: &Client,
        catalog: &CatalogPage,
        id: PaneId,
        size: Size,
    ) -> Result {
        let project = self.store_mut()?.state.active;
        let Some(mut pane) = self.store_mut()?.state.pane_mut(id).cloned() else {
            return Ok(());
        };
        if pane.error.is_some() {
            return Ok(());
        }
        if let Some(operation) = pane.creation {
            let directory = PathBuf::from(OsString::from_vec(pane.directory.0.clone()));
            match client.create_project_session(project, operation, &directory, size) {
                Ok(info) => {
                    self.store_mut()?.change(|state| {
                        let pane = state
                            .pane_mut(id)
                            .ok_or("Pane disappeared during creation")?;
                        pane.session = Some(info.id);
                        pane.creation = None;
                        Ok(())
                    })?;
                    pane.session = Some(info.id);
                }
                Err(ClientError::Server(error)) if error.code != ErrorCode::PersistenceFailed => {
                    self.store_mut()?.change(|state| {
                        if let Some(pane) = state.pane_mut(id) {
                            pane.creation = None;
                            pane.error = Some(error.message.chars().take(512).collect());
                        }
                        Ok(())
                    })?;
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        let Some(session) = pane.session else {
            return Ok(());
        };
        if Some(session) == hosting_session(catalog.server) {
            return Ok(());
        }
        let existing = lock(&self.shared).views.get(&id).cloned();
        if let Some(view) = existing {
            if view.ended {
                return Ok(());
            }
            if let Some(channel) = view.channel {
                if view.viewport != size {
                    if let Some(view) = lock(&self.shared).views.get_mut(&id) {
                        view.viewport = size;
                        view.grid.resize(size);
                    }
                    client
                        .resize(channel, size)
                        .map_err(|error| error.to_string())?;
                }
                return Ok(());
            }
        }
        let view = match client.attach(session, size) {
            Ok(attachment) => View::attached(session, size, attachment),
            Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownSession => {
                match client.read_saved_screen(session) {
                    Ok(screen) => View {
                        session,
                        channel: None,
                        grid: RunGrid::from_saved(screen),
                        process: None,
                        input: muxy_protocol::InputModes::default(),
                        title: "Ended terminal".into(),
                        viewport: size,
                        ended: true,
                    },
                    Err(ClientError::Server(error))
                        if error.code == ErrorCode::SavedContentUnavailable =>
                    {
                        self.store_mut()?.change(|state| {
                            if let Some(pane) = state.pane_mut(id) {
                                pane.error = Some("Terminal is no longer available".into());
                            }
                            Ok(())
                        })?;
                        return Ok(());
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Err(error) => return Err(error.to_string()),
        };
        let ack = lock(&self.shared).insert(id, view);
        if let Some((channel, seq)) = ack {
            client
                .ack(channel, seq)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn list_sessions(&self, client: &Client, catalog: &CatalogPage) -> Result {
        let project = self
            .store
            .as_ref()
            .ok_or("TUI state is not ready")?
            .state
            .active;
        let mut sessions = Vec::new();
        let mut after = None;
        let mut revision = None;
        loop {
            let page = client
                .project_sessions(project, after, revision)
                .map_err(|error| error.to_string())?;
            revision = Some(page.revision);
            sessions.extend(
                page.sessions
                    .into_iter()
                    .filter(|session| Some(session.info.id) != hosting_session(catalog.server)),
            );
            if sessions.len() > 4096 {
                return Err("Too many terminals for the picker".into());
            }
            after = page.next;
            if after.is_none() {
                break;
            }
        }
        lock(&self.shared).sessions = sessions;
        Ok(())
    }

    fn store_mut(&mut self) -> Result<&mut Store> {
        self.store
            .as_mut()
            .ok_or_else(|| "TUI state is not ready".into())
    }
    fn message(&self, text: &str) {
        lock(&self.shared).message = text.into();
    }
    fn publish(&self, catalog: &CatalogPage) {
        let mut shared = lock(&self.shared);
        shared.state = self.store.as_ref().map(|store| store.state.clone());
        shared.catalog = Some(catalog.clone());
    }
}

pub(crate) fn hosting_session(server: ServerIdentity) -> Option<SessionId> {
    let inherited: ServerIdentity = std::env::var("MUXY_SERVER_ID").ok()?.parse().ok()?;
    (inherited == server).then_some(())?;
    SessionId::new(std::env::var("MUXY_SESSION_ID").ok()?.parse().ok()?)
}
