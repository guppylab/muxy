use super::*;

#[gpui::test]
fn command_palette_clicks_execute_once_and_outside_click_dismisses(cx: &mut TestAppContext) {
    let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1000.0), px(600.0)));
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("new tab");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("command-palette").expect("palette");
    cx.simulate_click(
        bounds.origin + gpui::point(px(60.0), px(48.0)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.state.current_project().tabs.len(), 1);
    });
    cx.simulate_keystrokes("cmd-shift-p");
    cx.run_until_parked();
    cx.simulate_click(gpui::point(px(900.0), px(500.0)), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let model = view.read(cx);
        assert!(model.overlay.is_none());
        let pane = model
            .terminal(&model.active_pane().expect("active pane"))
            .expect("terminal");
        assert!(pane.view.read(cx).focus.is_focused(window));
    });
}

#[gpui::test]
fn command_palette_executes_workspace_actions_and_restores_terminal_focus(cx: &mut TestAppContext) {
    let (boot, _requests) = stub_boot(AppState::bootstrap().expect("state"));
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1000.0), px(600.0)));
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("new tab");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.state.current_project().tabs.len(), 1);
    });
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("split right");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.state.current_project().tabs[0].panes.len(), 2);
    });
    cx.simulate_keystrokes("cmd-shift-p escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let model = view.read(cx);
        let pane = model
            .terminal(&model.active_pane().expect("active pane"))
            .expect("terminal");
        assert!(pane.view.read(cx).focus.is_focused(window));
    });
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("change theme");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(matches!(model.overlay, Some(Overlay::Themes { .. })));
    });
}

#[gpui::test]
fn command_palette_navigates_projects_and_opens_settings_with_a_remapped_shortcut(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("project directory");
    let mut state = AppState::bootstrap().expect("state");
    let project = state
        .add_project(directory.path().to_path_buf())
        .expect("project");
    state
        .rename_project(project, "Palette Project")
        .expect("name");
    state.select_project(state.home().id).expect("home");
    let (mut boot, _requests) = stub_boot(state);
    boot.settings.keymap = boot
        .settings
        .keymap
        .with_binding(
            "toggle_command_palette",
            Some("cmd-shift-j".parse().expect("chord")),
        )
        .expect("keymap");
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_keystrokes("cmd-shift-p");
    cx.run_until_parked();
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
    cx.simulate_keystrokes("cmd-shift-j");
    cx.simulate_input("switch project");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("picker-back").is_some());
    cx.simulate_input("palette project");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.state.current_project().id, project);
        assert!(model.state.current_project().tabs.is_empty());
    });
    cx.simulate_keystrokes("cmd-shift-j cmd-shift-j");
    cx.run_until_parked();
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
    cx.simulate_keystrokes("cmd-shift-j");
    cx.simulate_input("open settings");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert!(model.settings_window.is_some());
        assert_eq!(model.state.current_project().id, project);
    });
}
