use super::*;
use crate::views::overlays::Overlay;

#[gpui::test]
fn font_dropdown_filters_saves_and_cancels_without_expanding_the_settings_rows(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Terminal");
    let section = cx
        .debug_bounds("settings-section-Terminal")
        .expect("terminal section");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    click_preference(cx, "settings-field-font-size");
    cx.simulate_keystrokes("cmd-a 2 1");
    settings.read_with(cx, |pane, cx| {
        assert_eq!(pane.field_value("font-size", cx), "21");
    });
    click_preference(cx, "settings-picker-font-family");
    view.read_with(cx, |model, _| {
        assert!(matches!(model.overlay, Some(Overlay::Fonts { .. })));
        assert_eq!(model.terminal.font_size, 21.0);
    });
    assert_eq!(cx.debug_bounds("settings-section-Terminal"), Some(section));
    assert!(
        cx.debug_bounds("settings-dropdown")
            .expect("font dropdown")
            .size
            .height
            <= px(360.0)
    );
    let font = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .max_by_key(String::len)
        .expect("available font");
    cx.simulate_input(&font);
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("settings-section-Terminal"), Some(section));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(
            model.terminal.font_families.first().map(String::as_str),
            Some(font.as_str())
        );
        let saved = muxy_settings::TerminalSettings::load_with_seed(
            &model.path.with_file_name("ghostty.conf"),
            None,
        )
        .expect("saved font");
        assert_eq!(saved.font_families, model.terminal.font_families);
    });
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
    click_preference(cx, "settings-picker-font-family");
    cx.simulate_keystrokes("z z z z z z enter");
    view.read_with(cx, |model, _| assert!(model.overlay.is_some()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(
            model.terminal.font_families.first().map(String::as_str),
            Some(font.as_str())
        );
    });
    let mut names = cx.text_system().all_font_names();
    names.retain(|name| !name.starts_with('.'));
    names.sort_unstable_by_key(|name| name.to_lowercase());
    names.dedup();
    click_preference(cx, "settings-picker-font-family");
    cx.simulate_keystrokes("down down up enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.terminal.font_families, [names[1].clone()]);
    });
}

#[gpui::test]
fn settings_theme_dropdowns_use_their_own_anchor_and_update_only_the_selected_mode(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let themes = boot.state_path.with_file_name("themes");
    std::fs::create_dir_all(&themes).expect("themes directory");
    std::fs::write(
        themes.join("Picker Fixture.conf"),
        "background = 123456\nforeground = abcdef\n",
    )
    .expect("test theme");
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    for (dark, selector) in [
        (true, "settings-picker-dark-theme"),
        (false, "settings-picker-light-theme"),
    ] {
        click_preference(cx, selector);
        let trigger = cx.debug_bounds(selector).expect("theme trigger");
        let dropdown = cx
            .debug_bounds("settings-dropdown")
            .expect("theme dropdown");
        assert_eq!(dropdown.top(), trigger.bottom() + px(4.0));
        assert_eq!(dropdown.right(), trigger.right());
        view.read_with(cx, |model, _| {
            let Some(Overlay::Themes {
                dark: target,
                source: Some(source),
                ..
            }) = &model.overlay
            else {
                panic!("settings theme picker")
            };
            assert_eq!(*target, dark);
            assert_eq!(source.anchor.get(), Some(trigger));
            assert_ne!(source.anchor.get(), model.theme_anchor);
        });
        cx.simulate_keystrokes("p i c k e r space f i x t u r e enter");
        cx.run_until_parked();
        view.read_with(cx, |model, _| {
            assert!(model.overlay.is_none());
            assert_eq!(
                if dark {
                    &model.appearance.dark_theme
                } else {
                    &model.appearance.light_theme
                },
                "Picker Fixture"
            );
            if dark {
                assert_eq!(model.appearance.light_theme, "Muxy Light");
            }
        });
    }
    cx.update(|window, cx| view.update(cx, |model, cx| model.open_theme_picker(window, cx)));
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        let Some(Overlay::Themes {
            source: None,
            anchor,
            ..
        }) = &model.overlay
        else {
            panic!("sidebar theme picker")
        };
        assert_eq!(*anchor, model.theme_anchor);
    });
    cx.simulate_keystrokes("m u x y enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(matches!(
            model.overlay,
            Some(Overlay::Themes { source: None, .. })
        ));
        assert_eq!(
            if model.dark {
                &model.appearance.dark_theme
            } else {
                &model.appearance.light_theme
            },
            "Muxy"
        );
    });
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
}

#[gpui::test]
fn settings_dropdowns_follow_their_trigger_on_resize_and_fit_the_window(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Terminal");
    click_preference(cx, "settings-picker-font-family");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for (width, height) in [
        (1200.0, 800.0),
        (650.0, 650.0),
        (460.0, 500.0),
        (1400.0, 900.0),
    ] {
        let renders = settings.read_with(cx, |pane, _| pane.render_count);
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        settings.read_with(cx, |pane, _| assert_eq!(pane.render_count - renders, 1));
        let trigger = cx
            .debug_bounds("settings-picker-font-family")
            .expect("font trigger");
        let dropdown = cx.debug_bounds("settings-dropdown").expect("font dropdown");
        view.read_with(cx, |model, _| {
            let source = model
                .overlay
                .as_ref()
                .and_then(Overlay::settings_source)
                .expect("open dropdown");
            assert_eq!(source.anchor.get(), Some(trigger));
        });
        assert!(dropdown.left() >= px(8.0) && dropdown.right() <= px(width - 8.0));
        assert!(dropdown.top() >= px(8.0) && dropdown.bottom() <= px(height - 8.0));
        assert!(dropdown.left() <= trigger.right() && dropdown.right() >= trigger.left());
        if trigger.bottom() + px(4.0) + dropdown.size.height <= px(height - 8.0) {
            assert_eq!(dropdown.top(), trigger.bottom() + px(4.0));
        }
    }
    view.update(cx, AppModel::new_tab);
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
}

#[gpui::test]
fn dropdown_from_an_inactive_settings_split_focuses_the_source_pane(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let settings = state.window().active_pane.expect("settings pane");
    state
        .split_pane(settings, Direction::Right)
        .expect("terminal split");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1600.0), px(850.0)));
    click_preference(cx, "settings-picker-dark-theme");
    view.read_with(cx, |model, _| {
        assert_eq!(model.active_pane(), Some(settings));
        assert_eq!(
            model
                .overlay
                .as_ref()
                .and_then(Overlay::settings_source)
                .expect("source")
                .pane,
            settings
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let settings = view.read_with(cx, |model, _| settings_view(model));
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
}

#[gpui::test]
fn font_dropdown_mouse_selection_reports_save_failure_and_outside_click_does_not_reopen(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let config = boot.state_path.with_file_name("ghostty.conf");
    std::fs::create_dir_all(&config).expect("blocked config path");
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Terminal");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    let previous = view.read_with(cx, |model, _| model.terminal.font_families.clone());
    let font = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .max_by_key(String::len)
        .expect("available font");
    click_preference(cx, "settings-picker-font-family");
    cx.simulate_input(&font);
    cx.run_until_parked();
    let dropdown = cx.debug_bounds("settings-dropdown").expect("font dropdown");
    cx.simulate_click(
        gpui::point(dropdown.center().x, dropdown.bottom() - px(16.0)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.terminal.font_families, previous);
    });
    settings.read_with(cx, |pane, _| {
        assert!(pane.errors.contains_key("font-family"));
    });
    std::fs::remove_dir(&config).expect("unblock config path");
    click_preference(cx, "settings-picker-font-family");
    cx.simulate_input(&font);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.overlay.is_none());
        assert_eq!(model.terminal.font_families, std::slice::from_ref(&font));
    });
    settings.read_with(cx, |pane, _| {
        assert!(!pane.errors.contains_key("font-family"));
    });
    click_preference(cx, "settings-picker-font-family");
    click_preference(cx, "settings-picker-font-family");
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
}

#[gpui::test]
fn search_result_dropdown_uses_the_visible_field_and_closes_when_scrolled_out(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(1200.0)));
    click_preference(cx, "settings-search");
    cx.simulate_keystrokes("t");
    click_preference(cx, "settings-picker-font-family");
    let trigger = cx
        .debug_bounds("settings-picker-font-family")
        .expect("font trigger");
    view.read_with(cx, |model, _| {
        let source = model
            .overlay
            .as_ref()
            .and_then(Overlay::settings_source)
            .expect("source");
        assert_eq!(source.anchor.get(), Some(trigger));
    });
    let settings = view.read_with(cx, |model, _| settings_view(model));
    settings.update(cx, |pane, cx| {
        let list = pane.results_state();
        list.scroll_to(gpui::ListOffset {
            item_ix: list.item_count() - 2,
            offset_in_item: px(0.0),
        });
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |model, _| assert!(model.overlay.is_none()));
    cx.update(|window, cx| assert!(settings.read(cx).focus.is_focused(window)));
    click_preference(cx, "settings-search");
    cx.simulate_keystrokes("cmd-a f o n t");
    cx.simulate_resize(size(px(650.0), px(650.0)));
    click_preference(cx, "settings-picker-font-family");
    let trigger = cx
        .debug_bounds("settings-picker-font-family")
        .expect("font trigger");
    let dropdown = cx.debug_bounds("settings-dropdown").expect("font dropdown");
    assert_eq!(dropdown.top(), trigger.bottom() + px(4.0));
    assert_eq!(dropdown.left(), trigger.left());
}
