use std::rc::Rc;

use gpui::{Action, AppContext, Context, Focusable, Window};
use muxy_core::shortcuts::ShortcutId;
use muxy_ui::command_palette::{Command, CommandPalette, CommandPaletteEvent, Registry};

use super::overlays::Overlay;
use crate::model::AppModel;

pub(crate) type Handler = Rc<dyn Fn(&mut AppModel, &mut Window, &mut Context<AppModel>)>;

pub(crate) fn action(
    model: &AppModel,
    id: ShortcutId,
    title: &'static str,
    action: impl Action,
) -> Command<Handler> {
    let handler: Handler = Rc::new(move |_, window, cx| {
        window.dispatch_action(action.boxed_clone(), cx);
    });
    let command = Command::new(id.name(), title, handler);
    match model.settings.keymap.chord(id) {
        Some(chord) => command.shortcut(
            gpui::Keystroke::parse(chord.as_str())
                .map_or_else(|_| chord.to_string(), |key| key.to_string()),
        ),
        None => command,
    }
}

impl AppModel {
    pub(crate) fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_prompt.is_some() {
            return;
        }
        if matches!(self.overlay, Some(Overlay::Commands(_))) {
            self.dismiss_overlay(cx);
            return;
        }
        let mut registry = Registry::default();
        super::workspace::register_commands(&mut registry, self);
        super::sidebar::register_commands(&mut registry, self, cx);
        super::settings::register_commands(&mut registry, self);
        let palette =
            cx.new(|cx| CommandPalette::new(registry, self.theme.clone(), self.metrics, cx));
        self.overlay_subscription =
            Some(
                cx.subscribe_in(&palette, window, |model, _, event, window, cx| {
                    model.dismiss_overlay(cx);
                    model.focus_active(window, cx);
                    model.focus_requested = false;
                    if let CommandPaletteEvent::Selected(handler) = event {
                        handler(model, window, cx);
                    }
                }),
            );
        palette.focus_handle(cx).focus(window);
        self.overlay = Some(Overlay::Commands(palette));
        cx.notify();
    }
}
