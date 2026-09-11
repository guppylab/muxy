#![allow(clippy::float_cmp)]

mod pickers;
mod resize;
mod window;

use super::*;
use crate::views::settings::{Change, SettingsEvent};
use muxy_core::shortcuts::ShortcutSettings;

fn settings_view(model: &AppModel) -> Entity<crate::views::settings::SettingsView> {
    model
        .settings_window
        .as_ref()
        .expect("settings window")
        .view
        .clone()
}

fn settings_root<'a>(
    model: &AppModel,
    cx: &'a gpui::App,
) -> &'a crate::views::settings::window::SettingsWindow {
    model
        .settings_window
        .as_ref()
        .expect("settings window")
        .window
        .read(cx)
        .expect("settings root")
}

fn settings_window(
    boot: Boot,
    cx: &mut TestAppContext,
) -> (Entity<AppModel>, &mut VisualTestContext) {
    let (model, main) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    main.update(|window, cx| model.update(cx, |model, cx| model.open_settings(window, cx)));
    let handle = model.read_with(main, |model, _| {
        model.settings_window.as_ref().expect("window").window
    });
    let settings = VisualTestContext::from_window(handle.into(), main).into_mut();
    settings.update(|window, _| window.activate_window());
    settings.run_until_parked();
    (model, settings)
}

#[gpui::test]
fn settings_shortcut_reuses_an_independent_window_without_a_server(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("terminal");
    let (boot, requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, main) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(main, AppModel::disconnect);
    let before = view.read_with(main, |model, _| model.state.clone());
    main.simulate_keystrokes("cmd-,");
    let handle = view.read_with(main, |model, _| {
        model.settings_window.as_ref().expect("settings").window
    });
    main.simulate_keystrokes("cmd-,");
    let settings = VisualTestContext::from_window(handle.into(), main).into_mut();
    settings.simulate_keystrokes("cmd-,");
    view.read_with(settings, |model, _| {
        assert_eq!(
            model.settings_window.as_ref().expect("settings").window,
            handle
        );
        assert_eq!(model.state, before);
    });
    assert_eq!(settings.windows().len(), 2);
    view.update(settings, |model, cx| {
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
            .expect("terminal"),
            model.terminal
        );
        assert_eq!(store::load(&model.path).expect("restore"), model.state);
    });
    settings.simulate_keystrokes("cmd-w");
    view.read_with(main, |model, _| {
        assert!(model.settings_window.is_none());
        assert_eq!(model.state, before);
        assert!(model.quitting == Quitting::Idle);
    });
    main.simulate_keystrokes("cmd-,");
    view.read_with(main, |model, _| {
        assert_ne!(
            model.settings_window.as_ref().expect("reopened").window,
            handle
        );
    });
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
    state.open_terminal_tab(state.home().id).expect("terminal");
    let terminal = state.window().active_pane.expect("terminal");
    let second = state.split_pane(terminal, Direction::Down).expect("split");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = settings_window(boot, cx);
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
        assert!(
            settings_view(model)
                .read(cx)
                .errors
                .contains_key("font-size")
        );
    });
}

#[gpui::test]
fn recorder_intercepts_app_actions_and_rebinding_keeps_widget_shortcuts(cx: &mut TestAppContext) {
    let state = AppState::bootstrap().expect("state");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
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
        assert_eq!(model.state.home().tabs.len(), 0);
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
        assert!(model.state.home().tabs.is_empty());
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
    let state = AppState::bootstrap().expect("state");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = settings_window(boot, cx);
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
        assert!(model.settings_window.is_some());
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
fn settings_fields_save_on_blur_window_close_and_quit(cx: &mut TestAppContext) {
    let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, settings) = settings_window(boot, cx);
    click_preference(settings, "settings-field-width");
    settings.simulate_keystrokes("cmd-a 1 2 0 0");
    click_preference(settings, "settings-field-height");
    view.read_with(settings, |model, _| {
        assert_eq!(model.settings.window.default_size[0], 1200.0);
    });
    settings.simulate_keystrokes("cmd-a 8 0 0");
    let main = view.read_with(settings, |model, _| model.window);
    assert!(settings.simulate_close());
    view.read_with(cx, |model, _| {
        assert!(model.settings_window.is_none());
        assert_eq!(model.settings.window.default_size[1], 800.0);
    });
    let main = VisualTestContext::from_window(main, cx).into_mut();
    main.simulate_keystrokes("cmd-,");
    let handle = view.read_with(main, |model, _| {
        model.settings_window.as_ref().expect("settings").window
    });
    let settings = VisualTestContext::from_window(handle.into(), main).into_mut();
    click_preference(settings, "settings-category-Terminal");
    click_preference(settings, "settings-field-font-size");
    settings.simulate_keystrokes("cmd-a 2 1");
    view.update(settings, AppModel::quit);
    view.read_with(settings, |model, _| {
        assert_eq!(model.terminal.font_size, 21.0);
    });
}

#[gpui::test]
fn invalid_queued_server_fields_do_not_stall_later_changes_or_allow_early_connect(
    cx: &mut TestAppContext,
) {
    let state = AppState::bootstrap().expect("state");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = settings_window(boot, cx);
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
fn workspace_actions_in_settings_do_not_change_terminal_tabs(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("terminal");
    let (boot, _) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    let before = view.read_with(cx, |model, _| model.state.clone());
    cx.simulate_keystrokes("cmd-t cmd-d cmd-shift-w cmd-1");
    view.read_with(cx, |model, _| assert_eq!(model.state, before));
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("font size");
    cx.run_until_parked();
    assert!(cx.debug_bounds("settings-field-font-size").is_some());
    view.read_with(cx, |model, cx| {
        assert!(
            !settings_view(model)
                .read(cx)
                .matching_setting_ids()
                .contains(&"width")
        );
    });
    cx.simulate_keystrokes("cmd-shift-e enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("settings-field-width").is_some());
}

#[gpui::test]
fn navigating_or_focusing_fields_cancels_recording_and_search_stays_control_sized(
    cx: &mut TestAppContext,
) {
    let state = AppState::bootstrap().expect("state");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Keyboard");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    cx.update(|window, cx| {
        settings.update(cx, |pane, cx| pane.begin_recording("new_tab", window, cx));
    });
    click_preference(cx, "settings-category-General");
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
        .debug_bounds("settings-category-General")
        .expect("category");
    assert!(search.size.height < px(40.0));
    assert!(categories.top() - search.bottom() < px(25.0));
}

#[gpui::test]
fn server_field_drafts_survive_queued_saves_failures_and_disconnection(cx: &mut TestAppContext) {
    let state = AppState::bootstrap().expect("state");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
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
fn settings_use_an_edge_to_edge_sidebar_and_stable_search_columns(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (_, cx) = settings_window(boot, cx);
    for width in [1800.0, 1040.0, 740.0, 460.0] {
        cx.simulate_resize(size(px(width), px(800.0)));
        click_preference(cx, "settings-category-General");
        let view = cx.debug_bounds("settings-view").expect("view");
        let container = cx.debug_bounds("settings-container").expect("container");
        let navigation = cx.debug_bounds("settings-navigation").expect("navigation");
        assert_eq!(view, container);
        assert_eq!(navigation.left(), view.left());
        assert_eq!(navigation.top(), view.top());
        let viewport = cx.debug_bounds("settings-sections").expect("viewport");
        let field = cx.debug_bounds("settings-field-width").expect("field");
        assert!(field.left() >= viewport.left() && field.right() <= viewport.right());
        if width >= 660.0 {
            assert_eq!(navigation.size.width, px(248.0));
            assert_eq!(navigation.bottom(), view.bottom());
            assert_eq!(viewport.left(), navigation.right() + px(32.0));
        } else {
            assert!(viewport.top() > navigation.bottom());
        }
        click_preference(cx, "settings-search");
        cx.simulate_input("width");
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("settings-container"), Some(container));
        let searched = cx
            .debug_bounds("settings-field-width")
            .expect("searched field");
        assert_eq!(searched.left(), field.left());
        assert_eq!(searched.right(), field.right());
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("no matching setting");
        cx.run_until_parked();
        let empty = cx.debug_bounds("settings-empty").expect("empty");
        assert_eq!(empty.left(), viewport.left());
        assert_eq!(empty.right(), viewport.right());
    }
}

#[gpui::test]
fn searching_or_changing_category_starts_at_the_top_of_the_settings_container(
    cx: &mut TestAppContext,
) {
    let state = AppState::bootstrap().expect("state");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
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
                click_preference(cx, "settings-category-General");
            }
            cx.run_until_parked();
            settings.read_with(cx, |pane, _| {
                let offset = pane.results_state().logical_scroll_top();
                assert_eq!(offset.item_ix, 0);
                assert_eq!(offset.offset_in_item, px(0.0));
            });
            assert_eq!(
                cx.debug_bounds("settings-section-General")
                    .expect("first result")
                    .origin,
                viewport.origin
            );
        }
    }
}

#[gpui::test]
fn subcategories_and_description_search_use_the_existing_settings(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    click_preference(cx, "settings-category-General");
    click_preference(cx, "settings-subcategory-Window size");
    view.read_with(cx, |model, cx| {
        assert_eq!(
            settings_view(model).read(cx).matching_setting_ids(),
            vec!["width", "height"]
        );
    });
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("clipboard selecting");
    cx.run_until_parked();
    view.read_with(cx, |model, cx| {
        assert_eq!(
            settings_view(model).read(cx).matching_setting_ids(),
            vec!["copy-on-select"]
        );
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("does not exist");
    cx.run_until_parked();
    view.read_with(cx, |model, cx| {
        assert!(
            settings_view(model)
                .read(cx)
                .matching_setting_ids()
                .is_empty()
        );
    });
}

#[gpui::test]
fn server_subsections_preserve_availability_messages_without_affecting_search(
    cx: &mut TestAppContext,
) {
    for connected in [false, true] {
        let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
        cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
        let (view, cx) = settings_window(boot, cx);
        view.update(cx, |model, cx| {
            model.connection = if connected {
                ConnectionState::Ready
            } else {
                ConnectionState::Disconnected
            };
            model.sync_preferences(cx);
        });
        click_preference(cx, "settings-disclosure-Server");
        click_preference(cx, "settings-subcategory-Sessions");
        assert!(cx.debug_bounds("settings-section-Server").is_some());
        assert!(cx.debug_bounds("settings-empty").is_none());
        assert!(cx.debug_bounds("settings-field-default-shell").is_none());

        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("does not exist");
        cx.run_until_parked();
        assert!(cx.debug_bounds("settings-empty").is_some());
    }
}

#[gpui::test]
fn settings_colors_follow_both_theme_modes_and_failed_theme_saves_are_visible(
    cx: &mut TestAppContext,
) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = settings_window(boot, cx);
    for (dark, name) in [(true, "Dracula"), (false, "Muxy Light")] {
        view.update(cx, |model, cx| {
            model.dark = dark;
            model.change_preference(Change::Theme(dark, name.into()), cx);
            let settings = settings_view(model);
            let theme = settings.read(cx).theme();
            assert_eq!(theme.bg, model.theme.bg);
            assert_eq!(theme.raised(), model.theme.raised());
            assert_eq!(theme.accent, model.theme.accent);
        });
    }
    view.update(cx, |model, cx| {
        let path = model.configuration_path("settings.toml");
        std::fs::remove_file(&path).expect("remove test file");
        std::fs::create_dir(&path).expect("block test save");
        let before = model.appearance.clone();
        model.change_preference(Change::Theme(false, "Dracula".into()), cx);
        assert_eq!(model.appearance, before);
        assert!(
            settings_view(model)
                .read(cx)
                .errors
                .contains_key("light-theme")
        );
    });
}

#[gpui::test]
fn settings_controls_are_reachable_and_activated_with_the_keyboard(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    // Navbar: General, Appearance, Keyboard, Terminal, Server; then the config button and toggle.
    cx.simulate_keystrokes("cmd-shift-e tab tab tab tab tab tab space");
    view.read_with(cx, |model, _| {
        assert!(!model.settings.window.confirm_running_process);
    });
    cx.simulate_keystrokes("tab cmd-a 1 1 0 0 enter");
    view.read_with(cx, |model, _| {
        assert_eq!(model.settings.window.default_size[0], 1100.0);
    });
}

#[gpui::test]
fn settings_show_quit_and_connection_failures_in_the_settings_window(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    view.update(cx, |model, _| model.server_preferences.busy = true);
    cx.simulate_keystrokes("cmd-q");
    view.read_with(cx, |model, cx| {
        assert!(model.quitting == Quitting::Idle);
        assert!(
            settings_view(model).read(cx).errors["application"].contains("Wait for the server")
        );
    });
    assert!(cx.debug_bounds("settings-application-error").is_some());
    view.update(cx, AppModel::disconnect);
    cx.dispatch_action(crate::views::workspace::EndAllSessionsAndQuit);
    cx.run_until_parked();
    view.read_with(cx, |model, cx| {
        assert!(model.quitting == Quitting::Idle);
        assert!(
            settings_view(model).read(cx).errors["application"].contains("Connect to the server")
        );
    });
    drop(requests);
    click_preference(cx, "settings-category-Server");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    settings.update(cx, |_, cx| cx.emit(SettingsEvent::Connect));
    cx.run_until_parked();
    view.read_with(cx, |model, cx| {
        assert!(
            settings_view(model).read(cx).errors["application"]
                .contains("connection worker stopped")
        );
    });
}

#[gpui::test]
fn keyboard_navigation_reveals_every_shortcut_in_both_directions(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = settings_window(boot, cx);
    click_preference(cx, "settings-category-Keyboard");
    cx.simulate_resize(size(px(900.0), px(500.0)));
    cx.simulate_keystrokes("cmd-shift-e tab tab tab tab tab tab");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for (index, shortcut) in muxy_core::shortcuts::ALL.iter().enumerate() {
        cx.update(|window, cx| {
            assert_eq!(
                settings.read(cx).focused_shortcut(window),
                Some((shortcut.id, false))
            );
        });
        let row = settings.read_with(cx, |settings, _| {
            settings
                .results_state()
                .bounds_for_item(index + 1)
                .expect("shortcut row")
        });
        let viewport = cx.debug_bounds("settings-sections").expect("viewport");
        assert!(
            row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
            "{}: {row:?} outside {viewport:?}",
            shortcut.id
        );
        cx.simulate_keystrokes("tab tab");
    }
    for shortcut in muxy_core::shortcuts::ALL.iter().rev() {
        cx.simulate_keystrokes("shift-tab shift-tab");
        cx.update(|window, cx| {
            assert_eq!(
                settings.read(cx).focused_shortcut(window),
                Some((shortcut.id, false))
            );
        });
    }
}

#[gpui::test]
fn tabbing_reveals_fields_below_a_short_settings_viewport(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (_, cx) = settings_window(boot, cx);
    cx.simulate_resize(size(px(740.0), px(480.0)));
    cx.simulate_keystrokes("cmd-shift-e tab tab tab tab tab tab tab tab");
    let row = cx
        .debug_bounds("settings-field-height")
        .expect("height field");
    let viewport = cx.debug_bounds("settings-sections").expect("viewport");
    assert!(
        row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
        "{row:?} outside {viewport:?}"
    );
}

#[gpui::test]
fn category_disclosures_and_content_use_the_real_setting_sections(cx: &mut TestAppContext) {
    let (boot, _) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = settings_window(boot, cx);
    assert!(
        cx.debug_bounds("settings-heading-Closing terminals")
            .is_some()
    );
    assert!(cx.debug_bounds("settings-heading-Window size").is_some());
    click_preference(cx, "settings-disclosure-Appearance");
    view.read_with(cx, |model, cx| {
        assert_eq!(
            settings_view(model).read(cx).matching_setting_ids(),
            vec!["confirm-process", "width", "height"]
        );
    });
    click_preference(cx, "settings-subcategory-Themes");
    view.read_with(cx, |model, cx| {
        assert_eq!(
            settings_view(model).read(cx).matching_setting_ids(),
            vec!["light-theme", "dark-theme"]
        );
    });
    assert!(cx.debug_bounds("settings-heading-Themes").is_some());
}
