use super::*;
use crate::components::ActivateButton;
use crate::theme::ColorScheme;
use gpui::{Context, FocusHandle, Render, TestAppContext};

struct Controls {
    focus: FocusHandle,
    activations: Vec<&'static str>,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::from_scheme(&ColorScheme::default());
        let metrics = Metrics::new(1.0);
        let style = Style {
            theme: &theme,
            metrics: &metrics,
        };
        div()
            .track_focus(&self.focus)
            .tab_group()
            .on_action(cx.listener(|view, _: &ActivateButton, _, _| {
                view.activations.push("bubbled");
            }))
            .children([
                button(
                    style,
                    "disabled",
                    "Disabled",
                    false,
                    cx.listener(|view, _, _, _| {
                        view.activations.push("disabled");
                    }),
                )
                .into_any_element(),
                button(
                    style,
                    "button",
                    "Button",
                    true,
                    cx.listener(|view, _, _, _| {
                        view.activations.push("button");
                    }),
                )
                .into_any_element(),
                toggle(
                    style,
                    "toggle",
                    false,
                    cx.listener(|view, _, _, _| {
                        view.activations.push("toggle");
                    }),
                ),
                picker_trigger(
                    style,
                    "picker",
                    "Picker",
                    None,
                    false,
                    cx.listener(|view, _, _, _| {
                        view.activations.push("picker");
                    }),
                ),
                segmented(
                    style,
                    "segmented",
                    &[
                        Choice {
                            enabled: false,
                            ..Choice::new("disabled", "Disabled")
                        },
                        Choice::new("segment", "Segment"),
                    ],
                    "segment",
                    cx.listener(|view, value: &SharedString, _, _| {
                        assert_eq!(value.as_ref(), "segment");
                        view.activations.push("segment");
                    }),
                ),
            ])
    }
}

#[gpui::test]
fn keyboard_activation_skips_disabled_controls_and_stops_at_the_control(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut registry = crate::shortcuts::Registry::new(&muxy_core::shortcuts::Defaults);
        crate::components::register_shortcuts(&mut registry);
        cx.bind_keys(registry.into_bindings());
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window);
        Controls {
            focus,
            activations: Vec::new(),
        }
    });
    cx.run_until_parked();
    for _ in 0..4 {
        cx.update(|window, _| window.focus_next());
        cx.simulate_keystrokes("enter space");
    }
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.activations,
            [
                "button", "button", "toggle", "toggle", "picker", "picker", "segment", "segment"
            ]
        );
    });
}
