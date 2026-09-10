use super::{Category, Change, SettingsEvent, SettingsPane};
use gpui::{AnyElement, Context, IntoElement, Keystroke, ParentElement, Styled, div, px};
use muxy_ui::controls;

pub(super) fn matching(pane: &SettingsPane) -> Vec<usize> {
    pane.shortcut_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| pane.matches(Category::Keyboard, name).then_some(index))
        .collect()
}

pub(super) fn row(pane: &SettingsPane, index: usize, cx: &mut Context<SettingsPane>) -> AnyElement {
    let id = muxy_core::shortcuts::ALL[index].id;
    let recording = pane.recording.as_deref() == Some(id);
    let chord = pane.snapshot.settings.keymap.binding(id);
    let label = if recording {
        "Press a shortcut…"
    } else {
        chord.map_or("Not assigned", muxy_settings::KeyChord::as_str)
    };
    let control = div()
        .flex()
        .flex_wrap()
        .gap(px(6.0))
        .child(controls::button(
            pane.style(),
            id,
            label,
            true,
            cx.listener(move |pane, _, window, cx| {
                pane.begin_recording(id, window, cx);
            }),
        ))
        .child(controls::button(
            pane.style(),
            &format!("reset-{id}"),
            "Reset",
            true,
            cx.listener(move |pane, _, _, cx| {
                pane.recording = None;
                cx.emit(SettingsEvent::Change(Change::Binding(id.into(), None)));
            }),
        ));
    pane.row(id, &pane.shortcut_names[index], control.into_any_element())
}

impl SettingsPane {
    pub(crate) fn begin_recording(
        &mut self,
        id: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.results.dirty = true;
        self.recording = Some(id.into());
        self.errors.remove(id);
        self.focus.focus(window);
        cx.notify();
    }

    pub(super) fn record_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(id) = self.recording.take() else {
            return;
        };
        self.results.dirty = true;
        cx.stop_propagation();
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            cx.notify();
            return;
        }
        match recorded_chord(keystroke) {
            Ok(chord) => cx.emit(SettingsEvent::Change(Change::Binding(id, Some(chord)))),
            Err(error) => {
                self.errors.insert(id, format!("{error}"));
            }
        }
        cx.notify();
    }
}

fn recorded_chord(keystroke: &Keystroke) -> muxy_settings::Result<muxy_settings::KeyChord> {
    let modifiers = keystroke.modifiers;
    let mut value = String::new();
    for (name, enabled) in [
        ("cmd", modifiers.platform),
        ("ctrl", modifiers.control),
        ("alt", modifiers.alt),
        ("shift", modifiers.shift),
        ("fn", modifiers.function),
    ] {
        if enabled {
            value.push_str(name);
            value.push('-');
        }
    }
    value.push_str(&keystroke.key);
    value.parse()
}
