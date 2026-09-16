use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    Subscription,
};
use muxy_ui::picker::{Picker, PickerConfig, PickerEvent, PickerItem, PickerRow, PickerStatus};
use muxy_ui::theme::{Metrics, Theme};

use crate::theme::Entry;

pub(crate) enum ThemeEvent {
    Selected(String),
    Dismiss,
}

pub(crate) struct ThemePicker {
    entries: Vec<Entry>,
    active: String,
    picker: Entity<Picker>,
    metrics: Metrics,
    _subscription: Subscription,
}

impl EventEmitter<ThemeEvent> for ThemePicker {}

impl Focusable for ThemePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).input().focus_handle(cx)
    }
}

impl ThemePicker {
    pub(crate) fn new(
        entries: Vec<Entry>,
        active: String,
        theme: Theme,
        metrics: Metrics,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| {
            Picker::new(
                PickerConfig::popover("theme-browser", "Search themes…"),
                theme,
                metrics,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |browser: &mut Self, _, event, cx| match event {
            PickerEvent::QueryChanged { query, .. } => browser.sync_picker(query, cx),
            PickerEvent::Confirmed(selection) | PickerEvent::SecondaryConfirmed(selection) => {
                if let Some(entry) = selection
                    .id
                    .strip_prefix("theme-")
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| browser.entries.get(index))
                {
                    cx.emit(ThemeEvent::Selected(entry.name.clone()));
                }
            }
            PickerEvent::Dismissed => cx.emit(ThemeEvent::Dismiss),
            _ => {}
        });
        let browser = Self {
            entries,
            active,
            picker,
            metrics,
            _subscription: subscription,
        };
        browser.sync_picker("", cx);
        browser
    }

    pub(crate) fn set_appearance(&mut self, active: String, theme: Theme, cx: &mut Context<Self>) {
        self.active = active;
        self.picker.update(cx, |picker, cx| {
            picker.set_appearance(theme, self.metrics, cx);
        });
        let query = self.picker.read(cx).query().to_owned();
        self.sync_picker(&query, cx);
    }

    fn sync_picker(&self, query: &str, cx: &mut Context<Self>) {
        let query = query.trim().to_lowercase();
        let items: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name.to_lowercase().contains(&query))
            .map(|(index, entry)| {
                let mut row = PickerRow::new(format!("theme-{index}"), entry.name.clone());
                row.current = entry.name == self.active;
                row.swatches = (0..16)
                    .filter_map(|slot| entry.scheme.palette_color(slot).map(Into::into))
                    .collect();
                PickerItem::Row(row)
            })
            .collect();
        let status = if items.is_empty() {
            PickerStatus::Empty("No themes found".into())
        } else {
            PickerStatus::Ready
        };
        self.picker.update(cx, |picker, cx| {
            picker.set_items(items, cx);
            picker.set_status(status, cx);
        });
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        self.picker.clone()
    }
}
