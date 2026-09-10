#![allow(clippy::float_cmp)]

mod pickers;
mod resize;

use super::*;
use crate::views::settings::{Change, SettingsEvent};
use muxy_core::shortcuts::ShortcutSettings;

fn settings_view(model: &AppModel) -> Entity<crate::views::settings::SettingsPane> {
    match &model.grids[&model.active_pane().expect("active pane")] {
        PaneView::Settings { view, .. } => view.clone(),
        PaneView::Terminal(_) => panic!("expected settings"),
    }
}

#[gpui::test]
fn settings_shortcut_dedupes_restores_and_works_without_a_server(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, AppModel::disconnect);
    cx.simulate_keystrokes("cmd-,");
    cx.simulate_keystrokes("cmd-,");
    view.update(cx, |model, cx| {
        assert_eq!(model.state.home().tabs.len(), 1);
        assert!(matches!(
            model.grids[&model.active_pane().expect("pane")],
            PaneView::Settings { .. }
        ));
        model.change_preference(Change::StatusBar(false), cx);
        model.change_preference(Change::Field("font-size", "19".into()), cx);
        assert!(!model.appearance.status_bar_visible);
        assert_eq!(model.terminal.font_size, 19.0);
        assert_eq!(
            muxy_settings::Settings::load(&model.path.with_file_name("settings.toml"))
                .expect("settings")
                .appearance,
            model.appearance
        );
        assert_eq!(
            muxy_settings::TerminalSettings::load_with_seed(
                &model.path.with_file_name("ghostty.conf"),
                None
            )
            .expect("terminal settings"),
            model.terminal
        );
        model.state = store::load(&model.path).expect("restore");
        model.grids.clear();
        model.sync_visible(cx);
        assert!(matches!(
            model.grids[&model.active_pane().expect("pane")],
            PaneView::Settings { .. }
        ));
    });
    cx.run_until_parked();
    assert!(requests.try_iter().all(|(_, work)| !matches!(
        work,
        Work::Attach { .. } | Work::ReadSaved { .. } | Work::Input(..)
    )));
}

#[gpui::test]
fn live_preferences_update_every_terminal_and_rejected_values_stay_unapplied(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let settings = state.window().active_pane.expect("settings pane");
    let terminal = state.split_pane(settings, Direction::Right).expect("split");
    let second = state.split_pane(terminal, Direction::Down).expect("split");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.change_preference(Change::Field("font-size", "23".into()), cx);
        model.change_preference(Change::Field("adjust-cell-height", "10%".into()), cx);
        model.change_preference(Change::CopyOnSelect(true), cx);
        for id in [terminal, second] {
            let pane = model.terminal(&id).expect("terminal").view.read(cx);
            assert_eq!(pane.terminal.font_size, 23.0);
            assert_eq!(
                pane.terminal.cell_height,
                muxy_settings::CellHeight::Percent(10.0)
            );
            assert!(pane.copy_on_select);
        }
        let before = std::fs::read(model.path.with_file_name("ghostty.conf")).expect("config");
        model.change_preference(Change::Field("font-size", "NaN".into()), cx);
        assert_eq!(model.terminal.font_size, 23.0);
        assert_eq!(
            std::fs::read(model.path.with_file_name("ghostty.conf")).expect("config"),
            before
        );
        let PaneView::Settings { view, .. } = &model.grids[&settings] else {
            panic!("settings");
        };
        assert!(view.read(cx).errors.contains_key("font-size"));
    });
}

#[gpui::test]
fn recorder_intercepts_app_actions_and_rebinding_keeps_widget_shortcuts(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.run_until_parked();
    let settings = view.read_with(cx, |model, _| settings_view(model));
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    cx.simulate_keystrokes("cmd-q");
    view.read_with(cx, |model, _| {
        assert!(model.quitting == Quitting::Idle);
    });
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    cx.simulate_keystrokes("cmd-v");
    settings.read_with(cx, |pane, _| assert!(pane.errors.contains_key("new_tab")));
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    cx.simulate_keystrokes("cmd-n");
    view.read_with(cx, |model, cx| {
        assert_eq!(model.state.home().tabs.len(), 1);
        assert_eq!(
            model
                .settings
                .keymap
                .binding("new_tab")
                .map(muxy_settings::KeyChord::as_str),
            Some("cmd-n"),
            "{:?}",
            settings.read(cx).errors
        );
        assert_eq!(
            model
                .settings
                .keymap
                .keys("text_input.copy", Some("TextInput")),
            vec!["cmd-c"]
        );
        assert_eq!(
            model
                .settings
                .keymap
                .keys("popover.dismiss", Some("CommandPopover")),
            vec!["escape"]
        );
        assert_eq!(
            model
                .settings
                .keymap
                .keys("menu.confirm_highlighted", Some("Menu")),
            vec!["enter"]
        );
    });
    cx.simulate_keystrokes("cmd-n");
    view.update(cx, |model, cx| {
        assert_eq!(model.state.home().tabs.len(), 2);
        model.change_preference(Change::Binding("new_tab".into(), None), cx);
        assert_eq!(
            model
                .settings
                .keymap
                .binding("new_tab")
                .map(muxy_settings::KeyChord::as_str),
            Some("cmd-t")
        );
    });
}

#[gpui::test]
fn server_control_confirms_and_restart_connects_only_after_successful_stop(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    view.update(cx, |model, _| model.connection = ConnectionState::Ready);
    settings.update(cx, |_, cx| {
        cx.emit(SettingsEvent::ServerControl { restart: true });
    });
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(
        requests
            .try_iter()
            .all(|(_, work)| !matches!(work, Work::StopServer { .. }))
    );
    settings.update(cx, |_, cx| {
        cx.emit(SettingsEvent::ServerControl { restart: true });
    });
    cx.run_until_parked();
    cx.simulate_prompt_answer("Restart");
    cx.run_until_parked();
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::StopServer { restart: true, .. }))
    );
    view.update(cx, |model, cx| {
        model.receive(
            (
                1,
                Update::ServerStopped {
                    restart: true,
                    result: Ok(()),
                },
            ),
            cx,
        );
    });
    assert!(
        requests
            .try_iter()
            .any(|(generation, work)| generation == 2 && matches!(work, Work::Connect))
    );
    view.read_with(cx, |model, _| {
        assert!(matches!(
            model.grids[&model.active_pane().expect("pane")],
            PaneView::Settings { .. }
        ));
    });
}

fn click_preference(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let position = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn settings_fields_save_on_blur_tab_switch_and_quit_and_fit_narrow_splits(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    let tab = state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(700.0), px(750.0)));
    click_preference(cx, "settings-field-width");
    cx.simulate_keystrokes("cmd-a 1 2 0 0");
    click_preference(cx, "settings-field-height");
    view.read_with(cx, |model, _| {
        assert_eq!(model.settings.window.default_size[0], 1200.0);
    });
    cx.simulate_keystrokes("cmd-a 8 0 0");
    view.update(cx, AppModel::new_tab);
    view.read_with(cx, |model, _| {
        assert_eq!(model.settings.window.default_size[1], 800.0);
    });
    view.update(cx, |model, cx| model.select_tab(tab, cx));
    click_preference(cx, "settings-category-Terminal");
    click_preference(cx, "settings-field-font-size");
    cx.simulate_keystrokes("cmd-a 2 1");
    view.update(cx, AppModel::quit);
    view.update(cx, |model, cx| {
        assert_eq!(model.terminal.font_size, 21.0);
        model.quitting = Quitting::Idle;
        model.split_pane(Direction::Right, cx);
    });
    cx.run_until_parked();
    let pane = cx.debug_bounds("settings-pane").expect("settings pane");
    let field = cx
        .debug_bounds("settings-field-font-size")
        .expect("font field");
    assert!(field.left() >= pane.left() && field.right() <= pane.right());
}

#[gpui::test]
fn invalid_queued_server_fields_do_not_stall_later_changes_or_allow_early_connect(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        let settings = muxy_protocol::ServerSettingsDoc { default_shell: None, history_budget_bytes: 1024 * 1024, shell_integration: true };
        model.server_preferences.document = Some(settings.clone());
        model.server_preferences.busy = true;
        model.change_preference(Change::Field("history-budget", "invalid".into()), cx);
        model.change_preference(Change::ShellIntegration(false), cx);
        model.receive_server_settings(Ok(settings), cx);
        assert!(model.server_preferences.busy);
        assert!(model.server_preferences.pending.is_empty());
        assert!(settings_view(model).read(cx).errors.contains_key("history-budget"));
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::WriteServerSettings(settings) if !settings.shell_integration)));
        model.server_preferences.control_busy = true;
        model.disconnect(cx);
        model.connect(cx);
        assert_eq!(model.generation, 1);
        assert!(requests.try_iter().all(|(_, work)| !matches!(work, Work::Connect)));
    });
}

#[gpui::test]
fn keyboard_tab_selection_refocuses_retained_settings(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    cx.simulate_keystrokes("cmd-t cmd-1");
    cx.run_until_parked();
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
    cx.simulate_keystrokes("cmd-2 cmd-[");
    cx.run_until_parked();
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
}

#[gpui::test]
fn navigating_or_focusing_fields_cancels_recording_and_search_stays_control_sized(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Keyboard");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    click_preference(cx, "settings-category-Appearance");
    click_preference(cx, "settings-field-width");
    cx.simulate_keystrokes("cmd-a 1 2 0 0 enter");
    view.read_with(cx, |model, _| {
        assert_eq!(model.settings.window.default_size[0], 1200.0);
        assert_eq!(model.settings.keymap, muxy_settings::Keymap::default());
    });
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    click_preference(cx, "settings-search");
    cx.simulate_keystrokes("w i d t h");
    view.read_with(cx, |model, _| {
        assert_eq!(model.settings.keymap, muxy_settings::Keymap::default());
    });
    let search = cx.debug_bounds("settings-search").expect("search");
    let categories = cx
        .debug_bounds("settings-category-Appearance")
        .expect("category");
    assert!(search.size.height < px(40.0));
    assert!(categories.top() - search.bottom() < px(25.0));
}

#[gpui::test]
fn server_field_drafts_survive_queued_saves_failures_and_disconnection(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.server_preferences.document = Some(muxy_protocol::ServerSettingsDoc {
            default_shell: None,
            history_budget_bytes: 1024 * 1024,
            shell_integration: true,
        });
        model.sync_preferences(cx);
    });
    click_preference(cx, "settings-category-Server");
    click_preference(cx, "settings-field-default-shell");
    cx.simulate_keystrokes("/ m i s s i n g enter");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    settings.read_with(cx, |pane, cx| {
        assert_eq!(pane.field_value("default-shell", cx), "/missing");
    });
    view.update(cx, |model, cx| {
        let original = model.server_preferences.document.clone().expect("settings");
        model.receive_server_settings(Ok(original.clone()), cx);
        assert!(model.server_preferences.busy);
        model.receive_server_settings(Err(io::Error::other("Not executable").into()), cx);
        assert_eq!(model.server_preferences.document, Some(original));
    });
    settings.read_with(cx, |pane, cx| {
        assert_eq!(pane.field_value("default-shell", cx), "/missing");
        assert!(pane.errors.contains_key("default-shell"));
    });
    click_preference(cx, "settings-field-default-shell");
    cx.simulate_keystrokes("cmd-a / b i n / b a s h enter");
    view.update(cx, AppModel::disconnect);
    settings.read_with(cx, |pane, cx| {
        assert_eq!(pane.field_value("default-shell", cx), "/bin/bash");
        assert!(pane.errors.contains_key("default-shell"));
    });
}

#[gpui::test]
fn settings_and_search_results_share_an_inset_width_limited_container(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (_view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    for width in [1800.0, 1200.0, 650.0, 460.0] {
        cx.simulate_resize(size(px(width), px(800.0)));
        click_preference(cx, "settings-category-Appearance");
        let pane = cx.debug_bounds("settings-pane").expect("pane");
        let container = cx.debug_bounds("settings-container").expect("container");
        let inset = px(if pane.size.width < px(660.0) {
            16.0
        } else {
            24.0
        });
        assert!(container.left() - pane.left() >= inset);
        assert!(pane.right() - container.right() >= inset);
        assert!(container.top() - pane.top() >= inset);
        assert!(pane.bottom() - container.bottom() >= inset);
        assert!(container.size.width <= px(1080.0));
        assert!((container.center().x - pane.center().x).abs() <= px(1.0));
        let section = cx
            .debug_bounds("settings-section-Appearance")
            .expect("section");
        let viewport = cx.debug_bounds("settings-sections").expect("viewport");
        assert_eq!(section.origin, viewport.origin);
        assert_eq!(section.right(), container.right());
        let field = cx
            .debug_bounds("settings-field-width")
            .expect("width field");
        assert!(field.left() >= section.left() && field.right() < section.right());

        click_preference(cx, "settings-search");
        cx.simulate_keystrokes("w i d t h");
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("settings-container"), Some(container));
        let result = cx
            .debug_bounds("settings-section-Appearance")
            .expect("result");
        assert_eq!(result.origin, section.origin);
        assert_eq!(result.size.width, section.size.width);
        let result_field = cx
            .debug_bounds("settings-field-width")
            .expect("result field");
        assert_eq!(result_field.left(), field.left());
        assert_eq!(result_field.right(), field.right());

        if pane.size.width < px(660.0) {
            let search = cx.debug_bounds("settings-search").expect("search");
            assert_eq!(result_field.left(), search.left());
            assert_eq!(result_field.right(), search.right());
        }

        cx.simulate_keystrokes("cmd-a f o n t");
        cx.run_until_parked();
        for selector in ["settings-section-Terminal", "settings-section-Keyboard"] {
            let result = cx.debug_bounds(selector).expect(selector);
            assert_eq!(result.left(), section.left());
            assert_eq!(
                result.right(),
                section.right(),
                "{selector} at window width {width}"
            );
        }

        cx.simulate_keystrokes("cmd-a z z z z z");
        cx.run_until_parked();
        let empty = cx.debug_bounds("settings-empty").expect("empty search");
        assert_eq!(empty.origin, section.origin);
        assert_eq!(empty.size.width, section.size.width);
    }
}

#[gpui::test]
fn searching_or_changing_category_starts_at_the_top_of_the_settings_container(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for width in [1200.0, 650.0] {
        cx.simulate_resize(size(px(width), px(650.0)));
        for search in [true, false] {
            click_preference(cx, "settings-category-Keyboard");
            let viewport = cx.debug_bounds("settings-sections").expect("viewport");
            cx.simulate_event(gpui::MouseMoveEvent {
                position: viewport.center(),
                ..Default::default()
            });
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: viewport.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-600.0))),
                ..Default::default()
            });
            cx.run_until_parked();
            settings.read_with(cx, |pane, _| {
                assert!(pane.results_state().logical_scroll_top().item_ix > 0);
            });
            if search {
                click_preference(cx, "settings-search");
                cx.simulate_keystrokes("t");
            } else {
                click_preference(cx, "settings-category-Appearance");
            }
            cx.run_until_parked();
            settings.read_with(cx, |pane, _| {
                let offset = pane.results_state().logical_scroll_top();
                assert_eq!(offset.item_ix, 0);
                assert_eq!(offset.offset_in_item, px(0.0));
            });
            assert_eq!(
                cx.debug_bounds("settings-section-Appearance")
                    .expect("first result")
                    .origin,
                viewport.origin
            );
        }
    }
}
