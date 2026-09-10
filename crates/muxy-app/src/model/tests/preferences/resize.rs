use super::*;

#[gpui::test]
#[ignore = "resize profiling, run explicitly with --ignored --nocapture"]
fn settings_resize_profile(cx: &mut TestAppContext) {
    use std::os::unix::fs::MetadataExt;

    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    let path = view.read_with(cx, |model, _| model.path.clone());
    for category in ["settings-category-Appearance", "settings-category-Keyboard"] {
        cx.simulate_resize(size(px(1200.0), px(800.0)));
        click_preference(cx, category);
        let initial_renders = settings.read_with(cx, |pane, _| pane.render_count);
        let mut inode = std::fs::metadata(&path).expect("state").ino();
        let mut writes = 0;
        let mut times = Vec::new();
        for frame in 0_u16..140 {
            let step = if frame < 70 { frame } else { 139 - frame };
            let width = 650.0 + f32::from(step) * 10.0;
            let start = Instant::now();
            cx.simulate_resize(size(px(width), px(800.0)));
            cx.run_until_parked();
            times.push(start.elapsed());
            let next = std::fs::metadata(&path).expect("state").ino();
            writes += usize::from(inode != next);
            inode = next;
        }
        let renders = settings.read_with(cx, |pane, _| pane.render_count) - initial_renders;
        let total: Duration = times.iter().sum();
        times.sort();
        eprintln!(
            "{category}: frames={}, renders={renders}, state_writes={writes}, total={total:?}, p50={:?}, p95={:?}",
            times.len(),
            times[times.len() / 2],
            times[times.len() * 95 / 100]
        );
    }
}

#[gpui::test]
fn responsive_resize_uses_one_render_and_only_visible_shortcut_rows(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    click_preference(cx, "settings-category-Keyboard");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for width in [1200.0, 650.0, 750.0, 680.0, 710.0, 460.0, 1800.0] {
        let (renders, rows) =
            settings.read_with(cx, |pane, _| (pane.render_count, pane.shortcut_row_count));
        cx.simulate_resize(size(px(width), px(650.0)));
        cx.run_until_parked();
        settings.read_with(cx, |pane, _| {
            assert_eq!(pane.render_count - renders, 1, "window width {width}");
            let rendered_rows = pane.shortcut_row_count - rows;
            assert!(
                rendered_rows > 0 && rendered_rows < muxy_core::shortcuts::ALL.len() / 2,
                "rendered {rendered_rows} rows at width {width}"
            );
            assert_eq!(
                pane.results_state().item_count(),
                muxy_core::shortcuts::ALL.len() + 3
            );
        });
        let pane = cx.debug_bounds("settings-pane").expect("pane");
        let category = cx
            .debug_bounds("settings-category-Keyboard")
            .expect("category");
        let viewport = cx.debug_bounds("settings-sections").expect("viewport");
        if pane.size.width - px(2.0) < px(660.0) {
            assert!(viewport.top() > category.bottom());
        } else {
            assert!(viewport.top() < category.top());
        }
    }
}

fn scroll_results(cx: &mut VisualTestContext, distance: f32) {
    let viewport = cx.debug_bounds("settings-sections").expect("viewport");
    cx.simulate_event(gpui::MouseMoveEvent {
        position: viewport.center(),
        ..Default::default()
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-distance))),
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn virtual_shortcuts_scroll_resize_and_remeasure_errors_without_losing_position(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(650.0)));
    click_preference(cx, "settings-category-Keyboard");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    scroll_results(cx, 600.0);
    let index = settings.read_with(cx, |pane, _| {
        pane.results_state().logical_scroll_top().item_ix
    });
    assert!(index > 1);
    cx.simulate_resize(size(px(650.0), px(650.0)));
    cx.run_until_parked();
    let before = settings.read_with(cx, |pane, _| {
        assert_eq!(pane.results_state().logical_scroll_top().item_ix, index);
        pane.results_state()
            .bounds_for_item(index)
            .expect("visible row")
    });
    let id = muxy_core::shortcuts::ALL[index - 1].id;
    settings.update(cx, |pane, cx| pane.set_error(id, Some("This shortcut conflicts with another action. Choose a different combination before saving."), cx));
    cx.run_until_parked();
    settings.read_with(cx, |pane, _| {
        assert_eq!(pane.results_state().logical_scroll_top().item_ix, index);
        let row = pane
            .results_state()
            .bounds_for_item(index)
            .expect("error row");
        let next = pane
            .results_state()
            .bounds_for_item(index + 1)
            .expect("following row");
        assert!(row.size.height > before.size.height);
        assert_eq!(row.bottom(), next.top());
    });
    cx.update(|window, cx| settings.update(cx, |pane, cx| pane.begin_recording(id, window, cx)));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    settings.read_with(cx, |pane, _| {
        assert!(!pane.errors.contains_key(id));
        assert_eq!(
            pane.results_state()
                .bounds_for_item(index)
                .expect("reset row")
                .size
                .height,
            before.size.height
        );
    });
}

#[gpui::test]
fn search_fields_keep_keyboard_focus_when_virtualized_offscreen(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(1200.0)));
    click_preference(cx, "settings-search");
    cx.simulate_keystrokes("t");
    click_preference(cx, "settings-field-font-size");
    cx.simulate_keystrokes("cmd-a 2 1");
    let focus = cx.update(|window, cx| window.focused(cx).expect("field focus"));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for _ in 0..5 {
        scroll_results(cx, 600.0);
    }
    settings.read_with(cx, |pane, _| {
        assert!(pane.results_state().logical_scroll_top().item_ix > 2);
    });
    for (width, height) in [(650.0, 650.0), (1200.0, 800.0)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        cx.update(|window, _| assert!(focus.is_focused(window)));
    }
    cx.simulate_keystrokes("cmd-a 2 3 enter");
    view.read_with(cx, |model, _| assert_eq!(model.terminal.font_size, 23.0));
}

#[gpui::test]
fn validation_resizes_the_virtual_section(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    cx.update(|cx| crate::views::workspace::bind_keys(&boot.settings.keymap, cx));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    click_preference(cx, "settings-category-Terminal");
    let settings = view.read_with(cx, |model, _| settings_view(model));
    let initial = settings.read_with(cx, |pane, _| {
        pane.results_state().bounds_for_item(0).expect("terminal")
    });
    click_preference(cx, "settings-field-font-size");
    cx.simulate_keystrokes("cmd-a 0 enter");
    cx.run_until_parked();
    settings.read_with(cx, |pane, _| {
        assert!(pane.errors.contains_key("font-size"));
        assert!(
            pane.results_state()
                .bounds_for_item(0)
                .expect("validation")
                .size
                .height
                > initial.size.height
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    settings.read_with(cx, |pane, _| {
        assert_eq!(
            pane.results_state()
                .bounds_for_item(0)
                .expect("reverted")
                .size
                .height,
            initial.size.height
        );
    });
}

#[gpui::test]
fn viewport_sized_wheel_scrolls_keep_the_full_distance_after_resize(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_settings_tab(state.home().id).expect("settings");
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    let settings = view.read_with(cx, |model, _| settings_view(model));
    for (width, height) in [(1200.0, 800.0), (650.0, 650.0), (1200.0, 950.0)] {
        click_preference(cx, "settings-category-Keyboard");
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        let distance = cx
            .debug_bounds("settings-sections")
            .expect("viewport")
            .size
            .height;
        let (heading_height, row_height) = settings.read_with(cx, |pane, _| {
            let list = pane.results_state();
            (
                list.bounds_for_item(0).expect("heading").size.height,
                list.bounds_for_item(1).expect("first shortcut").size.height,
            )
        });
        scroll_results(cx, f32::from(distance));
        settings.read_with(cx, |pane, _| {
            let offset = pane.results_state().logical_scroll_top();
            assert!(offset.item_ix > 0);
            let actual = heading_height
                + row_height
                    * f32::from(u16::try_from(offset.item_ix - 1).expect("shortcut index"))
                + offset.offset_in_item;
            assert!(
                (actual - distance).abs() <= px(1.0),
                "requested {distance:?}, moved {actual:?} at {width}x{height}"
            );
        });
        scroll_results(cx, -f32::from(distance));
        settings.read_with(cx, |pane, _| {
            let offset = pane.results_state().logical_scroll_top();
            assert_eq!(offset.item_ix, 0);
            assert_eq!(offset.offset_in_item, px(0.0));
        });
    }
}
