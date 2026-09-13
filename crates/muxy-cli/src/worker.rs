mod io;
pub(crate) use io::{InputWriter, Shared, View, lock};

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

use crate::input::Input;
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
    CheckClose,
    Close(PaneId),
    Existing(ProjectSession),
    ListSessions,
    Detach,
    Input(Input),
}

pub(crate) struct Worker {
    pub shared: Arc<Mutex<Shared>>,
    pub input: Arc<InputWriter>,
    pub viewport: Arc<Mutex<Rect>>,
    sender: SyncSender<Action>,
    stop: Arc<AtomicBool>,
    connection: Arc<Mutex<Option<Client>>>,
    thread: Option<JoinHandle<()>>,
    ordered: Arc<AtomicUsize>,
    queued_bytes: Arc<AtomicUsize>,
}

impl Worker {
    pub(crate) fn start(profile: PathBuf, executable: PathBuf, viewport: Rect) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let input = Arc::new(InputWriter::new(Arc::clone(&shared))?);
        let ordered = Arc::new(AtomicUsize::new(0));
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let viewport = Arc::new(Mutex::new(viewport));
        let stop = Arc::new(AtomicBool::new(false));
        let connection = Arc::new(Mutex::new(None));
        let (sender, receiver) = mpsc::sync_channel(1024);
        let mut core = Core {
            profile,
            executable,
            shared: Arc::clone(&shared),
            viewport: Arc::clone(&viewport),
            stop: Arc::clone(&stop),
            connection: Arc::clone(&connection),
            store: None,
            references: None,
            input: Arc::clone(&input),
            ordered: Arc::clone(&ordered),
            queued_bytes: Arc::clone(&queued_bytes),
        };
        let worker = thread::Builder::new()
            .name("muxy-tui-requests".into())
            .spawn(move || {
                core.run(&receiver);
                lock(&core.shared).exit.get_or_insert(Ok(()));
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
            ordered,
            queued_bytes,
        })
    }

    pub(crate) fn send(&self, action: Action) -> Result {
        if !matches!(action, Action::Detach)
            && lock(&self.shared)
                .client
                .as_ref()
                .is_none_or(|client| !client.is_connected())
        {
            return Err("Server is disconnected; the action was not applied".into());
        }
        let ordered = !matches!(action, Action::ListSessions);
        let bytes = match &action {
            Action::Input(input) => input.length(),
            _ => 0,
        };
        self.queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(bytes)
                    .filter(|count| *count <= 16 * muxy_protocol::MAX_INPUT)
            })
            .map_err(|_| "Input buffer is full; wait before sending more text")?;
        if ordered {
            self.ordered.fetch_add(1, Ordering::AcqRel);
        }
        if self.sender.try_send(action).is_err() {
            if ordered {
                self.ordered.fetch_sub(1, Ordering::AcqRel);
            }
            self.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
            return Err("TUI is busy; wait for the pending action".into());
        }
        Ok(())
    }

    pub(crate) fn typing(&self, input: Input) -> Result {
        if self.ordered.load(Ordering::Acquire) > 0 {
            return self.send(Action::Input(input));
        }
        send_input(&self.shared, &self.input, input)
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
    references: Option<(u64, Vec<SessionId>)>,
    input: Arc<InputWriter>,
    ordered: Arc<AtomicUsize>,
    queued_bytes: Arc<AtomicUsize>,
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
                    match requests.recv_timeout(Duration::from_millis(500)) {
                        Ok(Action::Detach) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(action) => {
                            if !matches!(action, Action::ListSessions) {
                                self.ordered.fetch_sub(1, Ordering::AcqRel);
                            }
                            if let Action::Input(input) = action {
                                self.queued_bytes
                                    .fetch_sub(input.length(), Ordering::AcqRel);
                            }
                            self.message(
                                "Server is disconnected; the pending action was not applied",
                            );
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
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
                    shared.message.clone_from(&error);
                    shared.exit = Some(Err(error));
                    break;
                }
            }
        }
    }

    fn connected(&mut self, client: &Client, requests: &Receiver<Action>) -> Result<bool> {
        self.references = None;
        let mut catalog = client.catalog().map_err(|error| error.to_string())?;
        if self.store.is_none() {
            self.store = Some(Store::load(&self.profile, &catalog)?);
        }
        self.store_mut()?
            .change(|state| state.reconcile(&catalog))?;
        self.publish(&catalog);
        lock(&self.shared).client = Some(client.clone());
        lock(&self.shared).refresh = true;
        self.message("");
        while !self.stop.load(Ordering::Acquire) && client.is_connected() {
            let refresh = {
                let mut shared = lock(&self.shared);
                std::mem::take(&mut shared.refresh)
            };
            if refresh || self.store_mut()?.state.catalog_revision > catalog.revision {
                match client.catalog() {
                    Ok(next) => {
                        self.store_mut()?.change(|state| state.reconcile(&next))?;
                        catalog = next;
                        self.publish(&catalog);
                        let picker = lock(&self.shared).session_picker;
                        if picker && let Err(error) = self.list_sessions(client, &catalog) {
                            self.message(&error);
                        }
                    }
                    Err(error) => self.message(&error.to_string()),
                }
            }
            if let Err(error) = self.synchronize(client, &catalog) {
                if self.store_mut()?.ready().is_err() {
                    return Err(error);
                }
                self.message(&error);
            }
            match requests.recv_timeout(Duration::from_millis(100)) {
                Ok(Action::Detach) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(true),
                Ok(action) => {
                    let ordered = !matches!(action, Action::ListSessions);
                    let bytes = match &action {
                        Action::Input(input) => input.length(),
                        _ => 0,
                    };
                    if let Err(error) = self.action(action, client, &catalog) {
                        self.message(&error);
                    }
                    self.publish(&catalog);
                    self.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
                    if ordered {
                        self.ordered.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(self.stop.load(Ordering::Acquire))
    }

    fn action(&mut self, action: Action, client: &Client, catalog: &CatalogPage) -> Result {
        if matches!(action, Action::CheckClose) {
            return self.check_close(client, catalog);
        }
        if let Action::Input(input) = action {
            return send_input(&self.shared, &self.input, input);
        }
        if matches!(action, Action::ListSessions) {
            return self.list_sessions(client, catalog);
        }
        self.input.flush()?;
        let host = hosting_session(catalog.server);
        let selection = self.store_mut()?.state.selection();
        let selected_tab = match action {
            Action::SelectTab(index) => self.store_mut()?.state.tab_selection(index),
            Action::CycleTab(forward) => self.store_mut()?.state.cycle_selection(forward),
            _ => None,
        };
        self.store_mut()?.change(|state| {
            if !matches!(action, Action::Open(_)) {
                state.select(selection)?;
            }
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
                Action::SelectTab(_) | Action::CycleTab(_) => {
                    if let Some(selected) = selected_tab {
                        state.select(selected)?;
                    }
                }
                Action::ListSessions | Action::Detach | Action::Input(_) | Action::CheckClose => {}
            }
            Ok(())
        })?;
        self.message("");
        Ok(())
    }

    fn check_close(&mut self, client: &Client, catalog: &CatalogPage) -> Result {
        self.input.flush()?;
        let tab = self.store_mut()?.state.tab().ok_or("No pane to close")?;
        let id = tab.focus;
        let session = tab.panes[&id].session;
        if session.is_some() && session == hosting_session(catalog.server) {
            return Err("Cannot close the terminal hosting this TUI".into());
        }
        if let Some(session) = session {
            let local = self
                .store_mut()?
                .state
                .projects
                .values()
                .flat_map(|project| &project.tabs)
                .flat_map(|tab| &tab.panes)
                .any(|(pane, value)| *pane != id && value.session == Some(session));
            if local {
                return self.action(Action::Close(id), client, catalog);
            }
            let size = lock(&self.shared)
                .views
                .get(&id)
                .map_or(Size { cols: 80, rows: 24 }, |view| view.viewport);
            match client.attach(session, size) {
                Ok(attachment) => {
                    let detached = client.detach(attachment.channel);
                    lock(&self.shared).retire(attachment.channel);
                    match detached {
                        Ok(()) => {}
                        Err(ClientError::Server(error))
                            if error.code == ErrorCode::UnknownChannel => {}
                        Err(error) => return Err(error.to_string()),
                    }
                    if attachment.process.is_none_or(|process| !process.is_shell) {
                        lock(&self.shared).confirm = Some(id);
                        return Ok(());
                    }
                }
                Err(ClientError::Server(error)) if error.code == ErrorCode::UnknownSession => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        self.action(Action::Close(id), client, catalog)
    }

    fn synchronize(&mut self, client: &Client, catalog: &CatalogPage) -> Result {
        self.store_mut()?.ready()?;
        self.sync_references(client)?;
        while let Some(discard) = self.store_mut()?.state.discards.first().copied() {
            match discard {
                Discard::Session(session) => client
                    .close_session(session, self.store_mut()?.state.close_operations[&session]),
                Discard::Creation(operation) => client.cancel_creation(operation),
            }
            .map_err(|error| error.to_string())?;
            self.store_mut()?.change(|state| {
                state.discards.retain(|pending| *pending != discard);
                Ok(())
            })?;
        }
        let viewport = *lock(&self.viewport);
        self.create_pending(client, viewport)?;
        self.sync_references(client)?;
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
                let focused = {
                    let mut shared = lock(&self.shared);
                    let focused = shared.focus == Some(channel);
                    if focused {
                        shared.focus = None;
                    }
                    focused
                };
                if focused {
                    self.input
                        .send(client.clone(), channel, b"\x1b[O".to_vec())?;
                }
                self.input.flush()?;
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

    fn sync_references(&mut self, client: &Client) -> Result {
        let state = &self.store_mut()?.state;
        let owner = state.layout_id.ok_or("TUI layout identity is missing")?;
        let revision = state.reference_revision;
        let references = state.session_references();
        if self.references.as_ref() != Some(&(revision, references.clone())) {
            if let Err(error) =
                client.sync_layout_references(Some(owner), revision, references.clone())
            {
                client.disconnect();
                return Err(error.to_string());
            }
            self.references = Some((revision, references));
        }
        Ok(())
    }

    fn create_pending(&mut self, client: &Client, viewport: Rect) -> Result {
        let state = self.store_mut()?.state.clone();
        let regions = crate::render::regions(&state, viewport);
        for (project, layout) in state.projects {
            for tab in layout.tabs {
                for (id, mut pane) in tab.panes {
                    if self.stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if pane.creation.is_none() || pane.error.is_some() {
                        continue;
                    }
                    let size = regions
                        .iter()
                        .find(|(pane, _)| *pane == id)
                        .and_then(|(_, rect)| crate::render::terminal_size(*rect))
                        .unwrap_or(Size { cols: 80, rows: 24 });
                    self.create_pane(client, project, id, size, &mut pane)?;
                }
            }
        }
        Ok(())
    }

    fn synchronize_pane(
        &mut self,
        client: &Client,
        catalog: &CatalogPage,
        id: PaneId,
        size: Size,
    ) -> Result {
        let Some(pane) = self.store_mut()?.state.pane_mut(id).cloned() else {
            return Ok(());
        };
        if pane.error.is_some() || pane.creation.is_some() {
            return Ok(());
        }
        let Some(session) = pane.session else {
            return Ok(());
        };
        if Some(session) == hosting_session(catalog.server) {
            return Ok(());
        }
        let existing = lock(&self.shared)
            .views
            .get(&id)
            .map(|view| (view.ended, view.channel, view.viewport));
        if let Some((ended, channel, viewport)) = existing {
            if ended {
                return Ok(());
            }
            if let Some(channel) = channel {
                if viewport != size {
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
        let mut view = match client.attach(session, size) {
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
        view.grid.graphics = muxy_protocol::Graphics::default();
        let ack = lock(&self.shared).insert(id, view);
        if let Some((channel, seq)) = ack {
            client
                .ack(channel, seq)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn create_pane(
        &mut self,
        client: &Client,
        project: ProjectId,
        id: PaneId,
        size: Size,
        pane: &mut crate::state::Pane,
    ) -> Result<bool> {
        if let Some(operation) = pane.creation {
            let directory = PathBuf::from(OsString::from_vec(pane.directory.0.clone()));
            match client.create_project_session(project, operation, &directory, size) {
                Ok(info) => {
                    self.store_mut()?.change(|state| {
                        if let Some(pane) = state.pane_mut(id)
                            && pane.creation == Some(operation)
                        {
                            pane.session = Some(info.id);
                            pane.creation = None;
                        }
                        Ok(())
                    })?;
                    pane.session = Some(info.id);
                }
                Err(ClientError::Server(error)) if error.code != ErrorCode::PersistenceFailed => {
                    self.store_mut()?.change(|state| {
                        if let Some(pane) = state.pane_mut(id)
                            && pane.creation == Some(operation)
                        {
                            pane.creation = None;
                            pane.error = Some(error.message.chars().take(512).collect());
                        }
                        Ok(())
                    })?;
                    return Ok(false);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(true)
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

fn send_input(shared: &Mutex<Shared>, writer: &InputWriter, input: Input) -> Result {
    let shared = lock(shared);
    let Some(view) = shared
        .state
        .as_ref()
        .and_then(|state| state.tab())
        .and_then(|tab| shared.views.get(&tab.focus))
        .filter(|view| !view.ended)
    else {
        return Ok(());
    };
    let Some((client, channel)) = shared.client.clone().zip(view.channel) else {
        return Ok(());
    };
    let modes = view.grid.modes;
    drop(shared);
    let bytes = input.encode(modes);
    if !bytes.is_empty() {
        writer.send(client, channel, bytes)?;
    }
    Ok(())
}

pub(crate) fn hosting_session(server: ServerIdentity) -> Option<SessionId> {
    let inherited: ServerIdentity = std::env::var("MUXY_SERVER_ID").ok()?.parse().ok()?;
    (inherited == server).then_some(())?;
    SessionId::new(std::env::var("MUXY_SESSION_ID").ok()?.parse().ok()?)
}
