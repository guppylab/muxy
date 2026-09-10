use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Context, InteractiveElement, IntoElement, ParentElement, Pixels, Styled, div, px,
};
use muxy_app_core::PaneId;
use muxy_ui::controls;

use super::{SettingsEvent, SettingsPane};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PickerKind {
    FontFamily,
    Theme(bool),
}

impl PickerKind {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::FontFamily => "font-family",
            Self::Theme(false) => "light-theme",
            Self::Theme(true) => "dark-theme",
        }
    }
}

pub(crate) type PickerAnchor = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Clone)]
pub(crate) struct PickerRequest {
    pub(crate) pane: PaneId,
    pub(crate) kind: PickerKind,
    pub(crate) anchor: PickerAnchor,
}

impl SettingsPane {
    pub(super) fn picker(
        &self,
        kind: PickerKind,
        value: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let anchor = self.picker_anchors[&kind].clone();
        let recorder = anchor.clone();
        div()
            .flex()
            .min_w(px(0.0))
            .debug_selector(move || format!("settings-picker-{}", kind.id()))
            .child(controls::picker_trigger(
                self.style(),
                kind.id(),
                value,
                (!self.compact).then_some(controls::CONTROL_WIDTH),
                false,
                cx.listener(move |pane, _, _, cx| {
                    pane.recording = None;
                    cx.emit(SettingsEvent::Picker(kind, anchor.clone()));
                }),
            ))
            .on_children_prepainted(move |bounds, window, _| {
                recorder.set(
                    bounds
                        .first()
                        .copied()
                        .filter(|bounds| window.content_mask().bounds.intersects(bounds)),
                );
            })
            .into_any_element()
    }
}

pub(crate) fn dropdown(
    picker: gpui::AnyElement,
    source: PickerRequest,
    cx: &mut Context<crate::model::AppModel>,
) -> gpui::AnyElement {
    let model = cx.weak_entity();
    gpui::canvas(
        move |_, window, cx| {
            let Some(anchor) = source.anchor.get() else {
                window.defer(cx, move |_, cx| {
                    let _ = model.update(cx, |model, cx| {
                        if model
                            .overlay
                            .as_ref()
                            .and_then(crate::views::overlays::Overlay::settings_source)
                            .is_some_and(|current| {
                                current.pane == source.pane && current.kind == source.kind
                            })
                        {
                            model.dismiss_overlay(cx);
                        }
                    });
                });
                return None;
            };
            let mut panel = div()
                .debug_selector(|| "settings-dropdown".into())
                .child(picker)
                .into_any_element();
            let size = panel.layout_as_root(
                gpui::size(
                    gpui::AvailableSpace::MinContent,
                    gpui::AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            let viewport = window.viewport_size();
            let left = if anchor.left() + size.width > viewport.width - px(8.0) {
                anchor.right() - size.width
            } else {
                anchor.left()
            };
            let below = anchor.bottom() + px(4.0);
            let above = anchor.top() - size.height - px(4.0);
            let top = if below + size.height > viewport.height - px(8.0) && above >= px(8.0) {
                above
            } else {
                below
            };
            let origin = crate::views::overlays::clamp(gpui::point(left, top), size, viewport);
            panel.prepaint_at(origin, window, cx);
            Some(panel)
        },
        |_, panel, window, cx| {
            if let Some(mut panel) = panel {
                panel.paint(window, cx);
            }
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}
