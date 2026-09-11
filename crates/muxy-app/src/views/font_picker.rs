use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    Subscription,
};
use muxy_ui::command_popover::{
    CommandPopover, CommandPopoverConfig, CommandPopoverDensity, CommandPopoverEvent,
    CommandPopoverItem, CommandPopoverPresentation, CommandPopoverRow, CommandPopoverStatus,
    CommandPopoverTab,
};
use muxy_ui::theme::{Metrics, Theme};

pub(crate) enum FontEvent {
    Selected(String),
    Dismiss,
}

pub(crate) struct FontPicker {
    names: Vec<String>,
    active: String,
    picker: Entity<CommandPopover>,
    _subscription: Subscription,
}

impl EventEmitter<FontEvent> for FontPicker {}

impl Focusable for FontPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).input().focus_handle(cx)
    }
}

impl FontPicker {
    pub(crate) fn new(
        active: String,
        theme: Theme,
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut names = cx.text_system().all_font_names();
        names.retain(|name| !name.starts_with('.'));
        names.sort_unstable_by_key(|name| name.to_lowercase());
        names.dedup();
        let picker = cx.new(|cx| {
            CommandPopover::new(
                CommandPopoverConfig {
                    id: "font-browser".into(),
                    presentation: CommandPopoverPresentation::Popover,
                    density: CommandPopoverDensity::Compact,
                    tabs: vec![CommandPopoverTab::new("fonts", "Fonts")],
                    placeholder: "Search fonts…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(340.0),
                    height: None,
                    max_height: Some(360.0),
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |browser: &mut Self, _, event, cx| match event {
            CommandPopoverEvent::QueryChanged { query, .. } => browser.sync_picker(query, cx),
            CommandPopoverEvent::Confirmed(selection)
            | CommandPopoverEvent::SecondaryConfirmed(selection) => {
                if let Some(name) = selection
                    .id
                    .strip_prefix("font-")
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| browser.names.get(index))
                {
                    cx.emit(FontEvent::Selected(name.clone()));
                }
            }
            CommandPopoverEvent::Dismissed => cx.emit(FontEvent::Dismiss),
            _ => {}
        });
        let browser = Self {
            names,
            active,
            picker,
            _subscription: subscription,
        };
        browser.sync_picker("", cx);
        browser
    }

    pub(crate) fn set_appearance(&self, theme: Theme, metrics: Metrics, cx: &mut Context<Self>) {
        self.picker
            .update(cx, |picker, cx| picker.set_appearance(theme, metrics, cx));
    }

    fn sync_picker(&self, query: &str, cx: &mut Context<Self>) {
        let query = query.trim().to_lowercase();
        let items: Vec<_> = self
            .names
            .iter()
            .enumerate()
            .filter(|(_, name)| name.to_lowercase().contains(&query))
            .map(|(index, name)| {
                let mut row = CommandPopoverRow::new(format!("font-{index}"), name.clone());
                row.current = *name == self.active;
                CommandPopoverItem::Row(row)
            })
            .collect();
        let status = if items.is_empty() {
            CommandPopoverStatus::Empty("No fonts found".into())
        } else {
            CommandPopoverStatus::Ready
        };
        self.picker.update(cx, |picker, cx| {
            picker.set_items(items, cx);
            picker.set_status(status, cx);
        });
    }
}

impl Render for FontPicker {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        self.picker.clone()
    }
}
