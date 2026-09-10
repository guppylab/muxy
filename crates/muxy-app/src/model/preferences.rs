use gpui::{AppContext, Context, Window};
use muxy_app_core::PaneId;
use muxy_settings::CellHeight;

use super::{AppModel, ConnectionState, PaneView, Quitting};
use crate::boot::Work;
use crate::views::font_picker::{FontEvent, FontPicker};
use crate::views::overlays::Overlay;
use crate::views::settings::{
    Change, PickerKind, PickerRequest, SettingsEvent, SettingsPane, Snapshot,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Default)]
pub(super) struct ServerPreferences {
    pub(super) document: Option<muxy_protocol::ServerSettingsDoc>,
    pub(super) busy: bool,
    pub(super) control_busy: bool,
    pub(super) row: String,
    pub(super) pending: std::collections::VecDeque<Change>,
}

impl AppModel {
    pub(crate) fn open_settings_picker(
        &mut self,
        request: PickerRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.visible_panes().contains(&request.pane) || request.anchor.get().is_none() {
            return;
        }
        self.flush_preferences(&[request.pane], cx);
        match request.kind {
            PickerKind::Theme(dark) => self.open_theme_picker_for(dark, Some(request), window, cx),
            PickerKind::FontFamily => {
                let active = self
                    .terminal
                    .font_families
                    .first()
                    .cloned()
                    .unwrap_or_default();
                let picker =
                    cx.new(|cx| FontPicker::new(active, self.theme.clone(), self.metrics, cx));
                self.overlay_subscription =
                    Some(cx.subscribe(&picker, |model, _, event, cx| match event {
                        FontEvent::Selected(name) => {
                            model.change_preference(Change::Field("font-family", name.clone()), cx);
                            model.dismiss_overlay(cx);
                        }
                        FontEvent::Dismiss => model.dismiss_overlay(cx),
                    }));
                self.overlay = Some(Overlay::Fonts {
                    picker,
                    source: request,
                });
                self.overlay_focus.focus(window);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quitting != Quitting::Idle || self.close_prompt.is_some() {
            return;
        }
        match self
            .state
            .open_settings_tab(self.state.current_project().id)
        {
            Ok(_) => {
                self.dismiss_overlay(cx);
                self.changed(cx);
                self.focus_active(window, cx);
                self.read_server_settings(cx);
            }
            Err(error) => self.fail(error.to_string(), cx),
        }
    }

    fn preferences_snapshot(&self) -> Snapshot {
        let mut settings = self.settings.clone();
        settings.appearance = self.appearance.clone();
        Snapshot {
            settings,
            terminal: self.terminal.clone(),
            server: self.server_preferences.document.clone(),
            connected: self.connection == ConnectionState::Ready,
            server_busy: self.server_preferences.busy || self.server_preferences.control_busy,
            pending_server_fields: self.pending_server_fields(),
        }
    }

    pub(super) fn pending_server_fields(&self) -> std::collections::HashSet<String> {
        self.server_preferences
            .pending
            .iter()
            .map(|change| change_id(change).to_owned())
            .chain(
                self.server_preferences
                    .busy
                    .then(|| self.server_preferences.row.clone()),
            )
            .collect()
    }

    pub(super) fn create_settings_view(&mut self, id: PaneId, cx: &mut Context<Self>) {
        let snapshot = self.preferences_snapshot();
        let included_keys = muxy_settings::TerminalSettings::included_keys(
            &self.path.with_file_name("ghostty.conf"),
        )
        .unwrap_or_default();
        let view = cx.new(|cx| {
            let mut pane = SettingsPane::new(snapshot, self.theme.clone(), self.metrics, cx);
            pane.set_included_keys(&included_keys);
            pane
        });
        let subscription = cx.subscribe(&view, move |model, _, event, cx| match event {
            SettingsEvent::Change(change) => model.change_preference(change.clone(), cx),
            SettingsEvent::Picker(kind, anchor) => {
                model.focus_pane(id, cx);
                model.settings_picker = Some(PickerRequest {
                    pane: id,
                    kind: *kind,
                    anchor: anchor.clone(),
                });
                cx.notify();
            }
            SettingsEvent::ServerControl { restart } => model.confirm_server_control(*restart, cx),
            SettingsEvent::ReadServer => model.read_server_settings(cx),
            SettingsEvent::Connect => model.connect(cx),
            SettingsEvent::Focused => {
                model.focus_pane(id, cx);
                model.focus_requested = false;
            }
        });
        self.grids.insert(
            id,
            PaneView::Settings {
                view,
                _subscription: subscription,
            },
        );
    }

    pub(super) fn flush_preferences(&mut self, ids: &[PaneId], cx: &mut Context<Self>) {
        let mut changes = Vec::new();
        for id in ids {
            if let Some(PaneView::Settings { view, .. }) = self.grids.get(id) {
                changes.extend(view.update(cx, |pane, cx| pane.take_changes(cx)));
            }
        }
        for change in changes {
            self.change_preference(change, cx);
        }
    }

    pub(super) fn preferences_before_quit(&mut self, cx: &mut Context<Self>) -> bool {
        self.flush_preferences(&self.grids.keys().copied().collect::<Vec<_>>(), cx);
        if self.server_preferences.busy || self.server_preferences.control_busy {
            self.fail(
                "Wait for the server settings operation to finish before quitting".into(),
                cx,
            );
            return false;
        }
        true
    }

    pub(super) fn sync_preferences(&self, cx: &mut Context<Self>) {
        let snapshot = self.preferences_snapshot();
        for pane in self.grids.values() {
            if let PaneView::Settings { view, .. } = pane {
                view.update(cx, |pane, cx| {
                    pane.sync(snapshot.clone(), self.theme.clone(), cx);
                });
            }
        }
    }

    pub(super) fn preference_result(&self, id: &str, error: Option<&str>, cx: &mut Context<Self>) {
        for pane in self.grids.values() {
            if let PaneView::Settings { view, .. } = pane {
                view.update(cx, |pane, cx| {
                    pane.set_error(id, error, cx);
                });
            }
        }
    }

    pub(super) fn change_preference(&mut self, change: Change, cx: &mut Context<Self>) {
        let id = change_id(&change).to_owned();
        if matches!(
            &change,
            Change::ShellIntegration(_) | Change::Field("default-shell" | "history-budget", _)
        ) {
            if self.server_preferences.busy {
                self.server_preferences
                    .pending
                    .retain(|pending| change_id(pending) != id);
                self.server_preferences.pending.push_back(change);
            } else if let Err(error) = self.write_server_preference(change, cx) {
                self.preference_result(&id, Some(&error.to_string()), cx);
            }
        } else {
            let result = self.apply_preference(change, cx);
            self.preference_result(
                &id,
                result.err().map(|error| error.to_string()).as_deref(),
                cx,
            );
        }
        self.sync_preferences(cx);
        cx.notify();
    }

    fn apply_preference(&mut self, change: Change, cx: &mut Context<Self>) -> Result<()> {
        let path = self.path.with_file_name("settings.toml");
        let mut settings = self.settings.clone();
        settings.appearance = self.appearance.clone();
        match change {
            Change::Sidebar(value) => {
                settings.appearance.sidebar_expanded = value;
                settings.appearance.save(&path)?;
            }
            Change::StatusBar(value) => {
                settings.appearance.status_bar_visible = value;
                settings.appearance.save(&path)?;
            }
            Change::ConfirmProcess(value) => {
                settings.window.confirm_running_process = value;
                settings.save_window(&path)?;
            }
            Change::CopyOnSelect(value) => {
                settings.clipboard.copy_on_select = value;
                settings.save_clipboard(&path)?;
            }
            Change::Directory(value) => {
                settings.panes.new_pane_directory = value;
                settings.save_panes(&path)?;
            }
            Change::Binding(id, chord) => {
                settings.keymap = settings.keymap.with_binding(&id, chord)?;
                settings.keymap.save(&path)?;
                cx.clear_key_bindings();
                crate::views::workspace::bind_keys(&settings.keymap, cx);
                cx.set_menus(crate::menus());
            }
            Change::Field(id @ ("width" | "height"), value) => {
                let index = usize::from(id == "height");
                settings.window.default_size[index] = value.parse()?;
                settings.save_window(&path)?;
            }
            Change::Field(id @ ("font-family" | "font-size" | "adjust-cell-height"), value) => {
                self.save_terminal_preference(id, &value, cx)?;
            }
            _ => return Err("Unknown app setting".into()),
        }
        self.appearance = settings.appearance.clone();
        self.settings = settings;
        for pane in self.grids.values().filter_map(PaneView::terminal) {
            pane.view.update(cx, |pane, cx| {
                pane.copy_on_select = self.settings.clipboard.copy_on_select;
                cx.notify();
            });
        }
        Ok(())
    }

    fn save_terminal_preference(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut requested = self.terminal.clone();
        match id {
            "font-family" => requested.font_families = vec![value.into()],
            "font-size" => requested.font_size = value.parse()?,
            "adjust-cell-height" => requested.cell_height = value.parse::<CellHeight>()?,
            _ => return Err("Unknown terminal setting".into()),
        }
        let effective = requested.save(&self.path.with_file_name("ghostty.conf"))?;
        let changed_size = self.terminal.font_size.to_bits() != effective.font_size.to_bits();
        self.terminal = effective;
        if changed_size {
            self.font_sizes.clear();
        }
        for pane in self.grids.values().filter_map(PaneView::terminal) {
            pane.view.update(cx, |pane, cx| {
                let zoom = pane.terminal.font_size;
                pane.terminal = self.terminal.clone();
                if !changed_size {
                    pane.terminal.font_size = zoom;
                }
                cx.notify();
            });
        }
        let included_keys = muxy_settings::TerminalSettings::included_keys(
            &self.path.with_file_name("ghostty.conf"),
        )?;
        for pane in self.grids.values() {
            if let PaneView::Settings { view, .. } = pane {
                view.update(cx, |pane, cx| {
                    pane.set_included_keys(&included_keys);
                    cx.notify();
                });
            }
        }
        Ok(())
    }

    pub(super) fn read_server_settings(&mut self, cx: &mut Context<Self>) {
        if self.connection == ConnectionState::Ready
            && !self.server_preferences.busy
            && !self.server_preferences.control_busy
        {
            self.server_preferences.busy = self.send(Work::ReadServerSettings, cx);
            self.server_preferences.row = "server".into();
            self.sync_preferences(cx);
        }
    }

    fn write_server_preference(&mut self, change: Change, cx: &mut Context<Self>) -> Result<()> {
        if self.connection != ConnectionState::Ready || self.server_preferences.control_busy {
            return Err("Connect to the server before editing its settings".into());
        }
        let mut settings = self
            .server_preferences
            .document
            .clone()
            .ok_or("Load server settings first")?;
        let id = change_id(&change).to_owned();
        match change {
            Change::ShellIntegration(value) => settings.shell_integration = value,
            Change::Field("default-shell", value) => {
                settings.default_shell =
                    (!value.is_empty()).then(|| muxy_protocol::ServerPath(value.into_bytes()));
            }
            Change::Field("history-budget", value) => {
                settings.history_budget_bytes = value
                    .parse::<u64>()?
                    .checked_mul(1024 * 1024)
                    .ok_or("History budget is too large")?;
            }
            _ => return Err("Unknown server setting".into()),
        }
        settings.validate().map_err(
            |_| "Use an absolute shell path and a history budget between 0 and 65536 MiB",
        )?;
        self.server_preferences.busy = self.send(Work::WriteServerSettings(settings), cx);
        self.server_preferences.row = id;
        if !self.server_preferences.busy {
            return Err("Could not send server settings".into());
        }
        Ok(())
    }

    pub(super) fn receive_server_settings(
        &mut self,
        result: std::result::Result<muxy_protocol::ServerSettingsDoc, muxy_client::ClientError>,
        cx: &mut Context<Self>,
    ) {
        self.server_preferences.busy = false;
        match result {
            Ok(settings) => {
                self.server_preferences.document = Some(settings);
                self.preference_result(&self.server_preferences.row, None, cx);
            }
            Err(error) => {
                self.preference_result(&self.server_preferences.row, Some(&error.to_string()), cx);
            }
        }
        while !self.server_preferences.busy {
            let Some(change) = self.server_preferences.pending.pop_front() else {
                break;
            };
            self.change_preference(change, cx);
        }
        self.sync_preferences(cx);
    }

    pub(super) fn receive_server_stopped(
        &mut self,
        restart: bool,
        result: std::result::Result<(), muxy_client::ClientError>,
        cx: &mut Context<Self>,
    ) {
        self.server_preferences.control_busy = false;
        match result {
            Ok(()) => {
                self.disconnect(cx);
                if restart {
                    self.connect(cx);
                }
            }
            Err(error) => self.preference_result(
                "server",
                Some(&format!("Could not stop server: {error}")),
                cx,
            ),
        }
        self.sync_preferences(cx);
    }

    fn confirm_server_control(&mut self, restart: bool, cx: &mut Context<Self>) {
        if self.close_prompt.is_some()
            || self.server_preferences.control_busy
            || self.server_preferences.busy
            || self.connection != ConnectionState::Ready
        {
            return;
        }
        self.dismiss_overlay(cx);
        let window = self.window;
        let generation = self.generation;
        self.close_prompt = Some(cx.spawn(async move |model, cx| {
            let response = crate::views::confirm::prompt_server(window, restart, cx).await;
            let _ = model.update(cx, |model, cx| {
                model.close_prompt = None;
                model.focus_requested = true;
                if generation == model.generation {
                    match response {
                        Ok(true) => {
                            model.server_preferences.control_busy = model.send(
                                Work::StopServer {
                                    socket: model.path.with_file_name("server.sock"),
                                    restart,
                                },
                                cx,
                            );
                            model.sync_preferences(cx);
                        }
                        Ok(false) => {}
                        Err(error) => model.preference_result("server", Some(&error), cx),
                    }
                }
                cx.notify();
            });
        }));
    }
}

fn change_id(change: &Change) -> &str {
    match change {
        Change::Sidebar(_) => "sidebar",
        Change::StatusBar(_) => "status-bar",
        Change::ConfirmProcess(_) => "confirm-process",
        Change::CopyOnSelect(_) => "copy-on-select",
        Change::Directory(_) => "directory",
        Change::Field(id, _) => id,
        Change::Binding(id, _) => id,
        Change::ShellIntegration(_) => "shell-integration",
    }
}
