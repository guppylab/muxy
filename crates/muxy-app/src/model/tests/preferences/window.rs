use super::*;
use crate::views::{settings::window::SettingsWindow, titlebar::BeginWindowMove, workspace::Zoom};
use gpui::{InteractiveElement, IntoElement, ParentElement, Render, Styled, VisualContext, div};

struct WindowActionObserver {
    settings: Entity<SettingsWindow>,
    move_requests: usize,
    zoom_requests: usize,
}

impl Render for WindowActionObserver {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .capture_action(cx.listener(|observer, _: &BeginWindowMove, _, cx| {
                observer.move_requests += 1;
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|observer, _: &Zoom, _, cx| {
                observer.zoom_requests += 1;
                cx.stop_propagation();
            }))
            .child(self.settings.clone())
    }
}

#[gpui::test]
fn settings_titlebars_route_window_movement_and_zoom_after_leaving_a_field(
    cx: &mut TestAppContext,
) {
    let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (model, cx) = settings_window(boot, cx);
    let observer = cx.replace_root_view(|window, _| WindowActionObserver {
        settings: window
            .root::<SettingsWindow>()
            .flatten()
            .expect("settings root"),
        move_requests: 0,
        zoom_requests: 0,
    });
    cx.run_until_parked();
    let before = model.read_with(cx, |model, _| model.state.clone());
    for (index, selector) in ["settings-sidebar-titlebar", "settings-content-titlebar"]
        .into_iter()
        .enumerate()
    {
        click_preference(cx, "settings-field-width");
        cx.simulate_keystrokes("cmd-a 1 1 0 0");
        model.read_with(cx, |model, cx| {
            assert_eq!(
                settings_view(model).read(cx).field_value("width", cx),
                "1100"
            );
        });
        let position = cx.debug_bounds(selector).expect("titlebar").center();
        for click_count in [1, 2] {
            cx.simulate_event(gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count,
                ..Default::default()
            });
            cx.simulate_event(gpui::MouseUpEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count,
                ..Default::default()
            });
            cx.run_until_parked();
        }
        observer.read_with(cx, |observer, _| {
            assert_eq!(observer.move_requests, index + 1, "{selector}");
            assert_eq!(observer.zoom_requests, index + 1, "{selector}");
        });
        model.read_with(cx, |model, _| {
            assert_eq!(model.settings.window.default_size[0], 1100.0);
            assert_eq!(model.state, before);
        });
    }
    for selector in [
        "settings-search",
        "settings-field-width",
        "settings-open-configuration",
    ] {
        let position = cx.debug_bounds(selector).expect("control").center();
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position: gpui::point(px(-1.0), px(-1.0)),
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        cx.run_until_parked();
        observer.read_with(cx, |observer, _| {
            assert_eq!(observer.move_requests, 2, "{selector}");
            assert_eq!(observer.zoom_requests, 2, "{selector}");
        });
    }
}

#[gpui::test]
fn settings_titlebar_click_saves_the_focused_field(cx: &mut TestAppContext) {
    let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (model, cx) = settings_window(boot, cx);
    click_preference(cx, "settings-field-width");
    cx.simulate_keystrokes("cmd-a 1 1 0 0");
    click_preference(cx, "settings-sidebar-titlebar");
    model.read_with(cx, |model, _| {
        assert_eq!(model.settings.window.default_size[0], 1100.0);
    });
}

#[gpui::test]
fn settings_close_shortcut_works_from_content_controls_and_pickers(cx: &mut TestAppContext) {
    for selectors in [
        vec![],
        vec!["settings-search"],
        vec!["settings-field-width"],
        vec!["settings-navigation"],
        vec!["settings-row-width"],
        vec!["settings-heading-Window size"],
        vec!["settings-sidebar-titlebar"],
        vec!["settings-content-titlebar"],
        vec!["settings-category-Appearance"],
        vec!["settings-category-Keyboard"],
        vec!["settings-category-Appearance", "settings-picker-dark-theme"],
        vec!["settings-category-Terminal", "settings-picker-font-family"],
    ] {
        let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
        cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
        let (model, cx) = settings_window(boot, cx);
        let before = model.read_with(cx, |model, _| model.state.clone());
        let count = cx.windows().len();
        for &selector in &selectors {
            click_preference(cx, selector);
        }
        cx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        model.read_with(cx, |model, _| {
            assert!(model.settings_window.is_none(), "focused {selectors:?}");
            assert_eq!(model.state, before);
        });
        assert_eq!(cx.windows().len(), count - 1);
    }
}

#[gpui::test]
fn settings_close_shortcut_works_after_keyboard_focus_is_cleared(cx: &mut TestAppContext) {
    for selectors in [
        vec![],
        vec!["settings-category-Appearance", "settings-picker-dark-theme"],
        vec!["settings-category-Terminal", "settings-picker-font-family"],
    ] {
        let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
        cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
        let (model, cx) = settings_window(boot, cx);
        for &selector in &selectors {
            click_preference(cx, selector);
        }
        let focused = cx.update(|window, cx| window.focused(cx).expect("focused control"));
        cx.update(|window, _| window.blur());
        cx.run_until_parked();
        cx.update(|window, _| assert!(focused.is_focused(window), "focused {selectors:?}"));
        cx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        model.read_with(cx, |model, _| assert!(model.settings_window.is_none()));
    }
}
