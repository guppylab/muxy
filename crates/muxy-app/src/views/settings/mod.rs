mod appearance;
mod keyboard;
mod pickers;
mod results;
mod server;
mod terminal;

pub(crate) use pickers::{PickerAnchor, PickerKind, PickerRequest, dropdown};

use std::collections::{HashMap, HashSet};

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, canvas, div, list, px,
};
use muxy_settings::{Settings, TerminalSettings};
use muxy_ui::controls::{self, Style};
use muxy_ui::text_input::{InputEvent, InputStyle, TextInput};
use muxy_ui::theme::{Metrics, Theme};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Category {
    Appearance,
    Terminal,
    Keyboard,
    Server,
}

impl Category {
    const ALL: [Self; 4] = [
        Self::Appearance,
        Self::Terminal,
        Self::Keyboard,
        Self::Server,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Keyboard => "Keyboard",
            Self::Server => "Server",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Change {
    Sidebar(bool),
    StatusBar(bool),
    ConfirmProcess(bool),
    CopyOnSelect(bool),
    Directory(muxy_settings::NewPaneDirectory),
    Field(&'static str, String),
    Binding(String, Option<muxy_settings::KeyChord>),
    ShellIntegration(bool),
}

pub(crate) enum SettingsEvent {
    Change(Change),
    Picker(PickerKind, PickerAnchor),
    ServerControl { restart: bool },
    ReadServer,
    Connect,
    Focused,
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) settings: Settings,
    pub(crate) terminal: TerminalSettings,
    pub(crate) server: Option<muxy_protocol::ServerSettingsDoc>,
    pub(crate) connected: bool,
    pub(crate) server_busy: bool,
    pub(crate) pending_server_fields: HashSet<String>,
}

pub(crate) struct SettingsPane {
    pub(crate) focus: FocusHandle,
    snapshot: Snapshot,
    theme: Theme,
    metrics: Metrics,
    search: Entity<TextInput>,
    query: String,
    results: results::Results,
    shortcut_names: Vec<String>,
    category: Category,
    fields: HashMap<&'static str, Entity<TextInput>>,
    dirty: HashSet<&'static str>,
    pub(crate) errors: HashMap<String, String>,
    pub(crate) notes: HashMap<String, String>,
    recording: Option<String>,
    focus_initialized: bool,
    compact: bool,
    picker_anchors: HashMap<PickerKind, PickerAnchor>,
    pub(crate) focus_outline: bool,
    subscriptions: Vec<Subscription>,
    #[cfg(test)]
    pub(crate) render_count: usize,
    #[cfg(test)]
    pub(crate) shortcut_row_count: usize,
}

impl EventEmitter<SettingsEvent> for SettingsPane {}

impl SettingsPane {
    pub(crate) fn new(
        snapshot: Snapshot,
        theme: Theme,
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            TextInput::new(InputStyle::field(&theme, &metrics), cx)
                .with_placeholder("Search settings")
        });
        let search_subscription = cx.subscribe(&search, |pane: &mut Self, _, event, cx| {
            if matches!(event, InputEvent::Changed) {
                pane.query = pane.search.read(cx).text().trim().to_lowercase();
                pane.results.reset();
            }
            cx.notify();
        });
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        let recorder = cx.intercept_keystrokes(move |event, window, cx| {
            let _ = weak.update(cx, |pane: &mut Self, cx| {
                if pane.focus.is_focused(window) {
                    pane.record_key(&event.keystroke, cx);
                }
            });
        });
        let mut pane = Self {
            focus,
            snapshot,
            theme,
            metrics,
            search,
            query: String::new(),
            results: results::Results::new(cx),
            shortcut_names: muxy_core::shortcuts::ALL
                .iter()
                .map(|shortcut| shortcut.id.replace(['_', '.'], " "))
                .collect(),
            category: Category::Appearance,
            fields: HashMap::new(),
            dirty: HashSet::new(),
            errors: HashMap::new(),
            notes: HashMap::new(),
            recording: None,
            focus_initialized: false,
            compact: false,
            picker_anchors: [
                PickerKind::FontFamily,
                PickerKind::Theme(false),
                PickerKind::Theme(true),
            ]
            .into_iter()
            .map(|kind| (kind, PickerAnchor::default()))
            .collect(),
            focus_outline: false,
            subscriptions: vec![search_subscription, recorder],
            #[cfg(test)]
            render_count: 0,
            #[cfg(test)]
            shortcut_row_count: 0,
        };
        for id in [
            "width",
            "height",
            "font-size",
            "adjust-cell-height",
            "default-shell",
            "history-budget",
        ] {
            let input =
                cx.new(|cx| TextInput::new(InputStyle::field(&pane.theme, &pane.metrics), cx));
            let changed = cx.subscribe(&input, move |pane: &mut Self, _, event, cx| match event {
                InputEvent::Changed => {
                    pane.dirty.insert(id);
                    pane.results.dirty = true;
                    cx.notify();
                }
                InputEvent::Submitted => pane.commit_field(id, cx),
                InputEvent::Cancelled => {
                    pane.dirty.remove(id);
                    pane.errors.remove(id);
                    pane.results.dirty = true;
                    pane.sync_fields(cx);
                    cx.notify();
                }
            });
            pane.fields.insert(id, input);
            pane.subscriptions.push(changed);
        }
        pane.sync_fields(cx);
        pane
    }

    pub(crate) fn sync(&mut self, snapshot: Snapshot, theme: Theme, cx: &mut Context<Self>) {
        self.results.dirty = true;
        self.snapshot = snapshot;
        self.theme = theme;
        let style = InputStyle::field(&self.theme, &self.metrics);
        self.search
            .update(cx, |input, cx| input.set_style(style, cx));
        for input in self.fields.values() {
            input.update(cx, |input, cx| input.set_style(style, cx));
        }
        self.sync_fields(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn field_value(&self, id: &str, cx: &gpui::App) -> String {
        self.fields[id].read(cx).text().to_owned()
    }

    fn sync_fields(&self, cx: &mut Context<Self>) {
        let settings = &self.snapshot.settings;
        let terminal = &self.snapshot.terminal;
        let server = self.snapshot.server.as_ref();
        let shell = server
            .and_then(|server| server.default_shell.as_ref())
            .map_or_else(String::new, |path| {
                String::from_utf8_lossy(&path.0).into_owned()
            });
        let budget = server.map_or_else(String::new, |server| {
            (server.history_budget_bytes / (1024 * 1024)).to_string()
        });
        for (id, value) in [
            ("width", settings.window.default_size[0].to_string()),
            ("height", settings.window.default_size[1].to_string()),
            ("font-size", terminal.font_size.to_string()),
            ("adjust-cell-height", terminal.cell_height.to_string()),
            ("default-shell", shell),
            ("history-budget", budget),
        ] {
            if !self.dirty.contains(id)
                && !self.errors.contains_key(id)
                && !self.snapshot.pending_server_fields.contains(id)
            {
                self.fields[id].update(cx, |input, cx| {
                    if input.text() != value {
                        input.set_text(value, cx);
                    }
                });
            }
        }
    }

    fn initialize_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_initialized {
            return;
        }
        self.focus_initialized = true;
        self.subscriptions
            .push(cx.on_focus_in(&self.focus, window, |_, _, cx| {
                cx.emit(SettingsEvent::Focused);
            }));
        self.subscriptions
            .push(cx.on_focus_out(&self.focus, window, |pane, _, _, cx| {
                pane.recording = None;
                pane.results.dirty = true;
                cx.notify();
            }));
        for input in std::iter::once(&self.search).chain(self.fields.values()) {
            self.subscriptions
                .push(cx.on_focus(&input.focus_handle(cx), window, |pane, _, cx| {
                    pane.recording = None;
                    pane.results.dirty = true;
                    cx.notify();
                }));
        }
        for (&id, input) in &self.fields {
            self.subscriptions.push(cx.on_blur(
                &input.focus_handle(cx),
                window,
                move |pane, _, cx| {
                    pane.results.dirty = true;
                    pane.commit_field(id, cx);
                    cx.notify();
                },
            ));
        }
    }

    pub(crate) fn take_changes(&mut self, cx: &Context<Self>) -> Vec<Change> {
        self.recording = None;
        self.results.dirty = true;
        self.dirty
            .drain()
            .map(|id| Change::Field(id, self.fields[id].read(cx).text().trim().to_owned()))
            .collect()
    }

    pub(crate) fn set_included_keys(&mut self, keys: &HashSet<String>) {
        self.results.dirty = true;
        self.notes.clear();
        for key in keys {
            self.notes.insert(key.clone(), "A config-file include supplies this setting. Edit the included file to change its value.".into());
        }
    }

    pub(crate) fn set_error(&mut self, id: &str, error: Option<&str>, cx: &mut Context<Self>) {
        if let Some(error) = error {
            self.errors.insert(id.into(), error.to_owned());
        } else {
            self.errors.remove(id);
        }
        self.results.dirty = true;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn results_state(&self) -> &gpui::ListState {
        &self.results.state
    }

    fn commit_field(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.dirty.remove(id) {
            let value = self.fields[id].read(cx).text().trim().to_owned();
            cx.emit(SettingsEvent::Change(Change::Field(id, value)));
        }
    }

    fn matches(&self, category: Category, label: &str) -> bool {
        let query = &self.query;
        if query.is_empty() {
            category == self.category
        } else {
            format!("{} {label}", category.label())
                .to_lowercase()
                .contains(query)
        }
    }

    fn style(&self) -> Style<'_> {
        Style {
            theme: &self.theme,
            metrics: &self.metrics,
        }
    }

    fn row(&self, id: &str, label: &str, control: AnyElement) -> AnyElement {
        let content = if self.compact {
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .px(self.metrics.spacing6())
                .py(self.metrics.spacing3())
                .gap(self.metrics.spacing2())
                .child(label.to_owned())
                .child(control)
                .into_any_element()
        } else {
            controls::row(self.style(), label, control)
        };
        div()
            .min_w(px(0.0))
            .child(content)
            .when_some(self.errors.get(id), |row, error| {
                row.child(self.note(error, true))
            })
            .when_some(self.notes.get(id), |row, note| {
                row.child(self.note(note, false))
            })
            .into_any_element()
    }

    fn note(&self, text: &str, error: bool) -> AnyElement {
        div()
            .px(self.metrics.spacing6())
            .pb(self.metrics.spacing3())
            .text_size(self.metrics.font_footnote())
            .text_color(if error {
                self.theme.danger
            } else {
                self.theme.fg_muted
            })
            .child(text.to_owned())
            .into_any_element()
    }

    fn field(&self, id: &'static str) -> AnyElement {
        let selector = format!("settings-field-{id}");
        div()
            .flex()
            .min_w(px(0.0))
            .debug_selector(move || selector.clone())
            .child(controls::text_field(
                self.style(),
                id,
                &self.fields[id],
                (!self.compact).then_some(210.0),
            ))
            .into_any_element()
    }

    fn toggle(
        &self,
        id: &'static str,
        value: bool,
        change: Change,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        controls::toggle(
            self.style(),
            id,
            value,
            cx.listener(move |_, _, _, cx| {
                cx.emit(SettingsEvent::Change(change.clone()));
            }),
        )
    }
}

impl SettingsPane {
    fn categories(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut categories = div().flex().gap(px(4.0)).map(|nav| {
            if self.compact {
                nav.flex_wrap()
            } else {
                nav.flex_col()
            }
        });
        for category in Category::ALL {
            categories = categories.child(
                div()
                    .id(SharedString::from(format!(
                        "settings-category-{}",
                        category.label()
                    )))
                    .debug_selector(move || format!("settings-category-{}", category.label()))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .when(self.category == category, |row| {
                        row.bg(self.theme.accent_soft)
                    })
                    .hover(|row| row.bg(self.theme.hover))
                    .child(category.label())
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.recording = None;
                        pane.category = category;
                        pane.query.clear();
                        pane.results.reset();
                        pane.search.update(cx, |search, cx| search.set_text("", cx));
                        pane.focus.focus(window);
                        if category == Category::Server {
                            cx.emit(SettingsEvent::ReadServer);
                        }
                        cx.notify();
                    })),
            );
        }
        categories.into_any_element()
    }
}

impl SettingsPane {
    fn navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .map(|nav| {
                if self.compact {
                    nav.w_full().border_b_1()
                } else {
                    nav.w(px(180.0)).h_full().border_r_1()
                }
            })
            .flex()
            .flex_col()
            .p(self.metrics.spacing6())
            .gap(px(14.0))
            .border_color(self.theme.border)
            .child(
                div()
                    .text_size(px(17.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .min_w(px(0.0))
                    .h(self.metrics.control_medium())
                    .debug_selector(|| "settings-search".into())
                    .child(controls::text_field(
                        self.style(),
                        "search",
                        &self.search,
                        None,
                    )),
            )
            .child(self.categories(cx))
            .into_any_element()
    }

    fn content(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity();
        div()
            .id("settings-sections")
            .debug_selector(|| "settings-sections".into())
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .map(|body| {
                if self.compact {
                    body.w_full()
                } else {
                    body.h_full()
                }
            })
            .overflow_hidden()
            .child(
                list(self.results.state.clone(), move |index, window, cx| {
                    view.update(cx, |pane, cx| pane.result(index, window, cx))
                })
                .size_full(),
            )
            .into_any_element()
    }

    fn layout_content(
        &mut self,
        size: gpui::Size<gpui::Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        for anchor in self.picker_anchors.values() {
            anchor.set(None);
        }
        self.compact = size.width < px(660.0);
        self.refresh_results(size.height, window, cx);
        let inset = if self.compact {
            self.metrics.spacing7()
        } else {
            self.metrics.spacing9()
        };
        div()
            .size_full()
            .flex()
            .justify_center()
            .p(inset)
            .child(
                div()
                    .debug_selector(|| "settings-container".into())
                    .w_full()
                    .max_w(self.metrics.scaled(1080.0))
                    .h_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .flex()
                    .when(self.compact, Styled::flex_col)
                    .child(self.navigation(cx))
                    .child(self.content(cx)),
            )
            .into_any_element()
    }
}

impl Render for SettingsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        self.initialize_focus(window, cx);
        let view = cx.entity();
        div()
            .id("settings-pane")
            .debug_selector(|| "settings-pane".into())
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .overflow_hidden()
            .border_1()
            .border_color(if self.focus_outline {
                self.theme.accent
            } else {
                self.theme.bg
            })
            .bg(self.theme.bg)
            .text_color(self.theme.fg)
            .text_size(self.metrics.font_body())
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let mut content = view
                            .update(cx, |pane, cx| pane.layout_content(bounds.size, window, cx));
                        content.layout_as_root(bounds.size.into(), window, cx);
                        content.prepaint_at(bounds.origin, window, cx);
                        content
                    },
                    |_, mut content, window, cx| content.paint(window, cx),
                )
                .size_full(),
            )
    }
}
