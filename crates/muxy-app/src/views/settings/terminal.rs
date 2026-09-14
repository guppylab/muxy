use super::{Category, Change, PickerKind, SettingsEvent, SettingsView};
use gpui::{AnyElement, Context, Window};
use muxy_app_core::settings::NewPaneDirectory;
use muxy_ui::controls::{self, Choice};

pub(super) fn rows(
    pane: &SettingsView,
    _: &Window,
    cx: &mut Context<SettingsView>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    if pane.matches(Category::Terminal, "Font family") {
        rows.push(
            pane.row(
                "font-family",
                "Font family",
                pane.picker(
                    PickerKind::FontFamily,
                    pane.snapshot
                        .terminal
                        .font_families
                        .first()
                        .map_or("Default", String::as_str),
                    cx,
                ),
            ),
        );
    }
    for (id, label) in [
        ("font-size", "Font size (points)"),
        ("adjust-cell-height", "Cell height adjustment (pixels or %)"),
    ] {
        if pane.matches(Category::Terminal, label) {
            rows.push(pane.row(id, label, pane.field(id)));
        }
    }
    let clipboard = pane.snapshot.settings.clipboard.copy_on_select;
    if pane.matches(Category::Terminal, "Copy on select") {
        rows.push(pane.row(
            "copy-on-select",
            "Copy on select",
            pane.toggle(
                "copy-on-select",
                clipboard,
                Change::CopyOnSelect(!clipboard),
                cx,
            ),
        ));
    }
    if pane.matches(Category::Terminal, "New pane directory") {
        let selected = match pane.snapshot.settings.panes.new_pane_directory {
            NewPaneDirectory::Project => "project",
            NewPaneDirectory::Current => "current",
        };
        rows.push(pane.row(
            "directory",
            "New pane directory",
            controls::segmented(
                pane.style(),
                "directory",
                &[
                    Choice::new("project", "Project"),
                    Choice::new("current", "Current pane"),
                ],
                selected,
                cx.listener(|_, selected: &gpui::SharedString, _, cx| {
                    cx.emit(SettingsEvent::Change(Change::Directory(
                        if selected.as_ref() == "current" {
                            NewPaneDirectory::Current
                        } else {
                            NewPaneDirectory::Project
                        },
                    )));
                }),
            ),
        ));
    }
    rows
}
