use super::{Category, Change, PickerKind, SettingsView};
use gpui::{AnyElement, Context};

pub(super) fn rows(
    pane: &SettingsView,
    category: Category,
    cx: &mut Context<SettingsView>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    let appearance = &pane.snapshot.settings.appearance;
    for (dark, label, value) in [
        (false, "Light theme", &appearance.light_theme),
        (true, "Dark theme", &appearance.dark_theme),
    ] {
        if category == Category::Appearance && pane.matches(category, label) {
            rows.push(pane.row(
                if dark { "dark-theme" } else { "light-theme" },
                label,
                pane.picker(PickerKind::Theme(dark), value, cx),
            ));
        }
    }
    for (id, label, value, change) in [
        (
            "sidebar",
            "Expand sidebar",
            appearance.sidebar_expanded,
            Change::Sidebar(!appearance.sidebar_expanded),
        ),
        (
            "status-bar",
            "Show status bar",
            appearance.status_bar_visible,
            Change::StatusBar(!appearance.status_bar_visible),
        ),
        (
            "confirm-process",
            "Confirm before closing a running process",
            pane.snapshot.settings.window.confirm_running_process,
            Change::ConfirmProcess(!pane.snapshot.settings.window.confirm_running_process),
        ),
    ] {
        let target = if id == "confirm-process" {
            Category::General
        } else {
            Category::Appearance
        };
        if category == target && pane.matches(category, label) {
            rows.push(pane.row(id, label, pane.toggle(id, value, change, cx)));
        }
    }
    for (id, label) in [
        ("width", "Default window width"),
        ("height", "Default window height"),
    ] {
        if category == Category::General && pane.matches(category, label) {
            rows.push(pane.row(id, label, pane.field(id)));
        }
    }
    rows
}
