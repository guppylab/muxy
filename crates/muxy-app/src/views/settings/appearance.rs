use super::{Category, Change, PickerKind, SettingsPane};
use gpui::{AnyElement, Context};

pub(super) fn rows(pane: &SettingsPane, cx: &mut Context<SettingsPane>) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    let appearance = &pane.snapshot.settings.appearance;
    for (dark, label, value) in [
        (false, "Light theme", &appearance.light_theme),
        (true, "Dark theme", &appearance.dark_theme),
    ] {
        if pane.matches(Category::Appearance, label) {
            rows.push(pane.row(
                label,
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
        if pane.matches(Category::Appearance, label) {
            rows.push(pane.row(id, label, pane.toggle(id, value, change, cx)));
        }
    }
    for (id, label) in [
        ("width", "Default window width"),
        ("height", "Default window height"),
    ] {
        if pane.matches(Category::Appearance, label) {
            rows.push(pane.row(id, label, pane.field(id)));
        }
    }
    if !rows.is_empty() {
        rows.push(pane.note(
            "Default dimensions are used when no saved window bounds exist.",
            false,
        ));
    }
    rows
}
