use super::*;

fn ready(model: &mut AppModel) {
    model.connection = ConnectionState::Ready;
    model.updates.ready = Some(crate::updater::PreparedUpdate::fixture().expect("update"));
}

#[gpui::test]
fn update_confirmation_cancels_or_flushes_before_stopping_the_server(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("tab");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.check_for_updates(true, cx);
        model.check_for_updates(true, cx);
    });
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate(_) | Work::Flush))
    );
    view.update(cx, |model, cx| model.check_for_updates(true, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update and Restart");
    cx.run_until_parked();
    let work: Vec<_> = requests.try_iter().collect();
    assert!(work.iter().any(|(_, work)| matches!(work, Work::Flush)));
    assert!(
        !work
            .iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate(_)))
    );
    view.update(cx, |model, cx| model.receive((1, Update::Flushed), cx));
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate(_)))
    );
    view.read_with(cx, |model, _| {
        assert!(model.quitting == Quitting::Update);
        assert_eq!(model.state.home().tabs.len(), 1);
    });
}

#[gpui::test]
fn failed_server_shutdown_retains_update_tabs_and_retry(cx: &mut TestAppContext) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("tab");
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.begin_update(cx);
        let before = model.state.clone();
        model.receive(
            (
                1,
                Update::StoppedForInstall(Err(muxy_client::ClientError::Timeout)),
            ),
            cx,
        );
        assert!(model.quitting == Quitting::Idle);
        assert!(model.updates.ready.is_some());
        assert_eq!(model.state, before);
        assert!(
            model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Could not prepare"))
        );
        model.begin_update(cx);
        assert!(model.quitting == Quitting::Update);
    });
    assert_eq!(
        requests
            .try_iter()
            .filter(|(_, work)| matches!(work, Work::Flush))
            .count(),
        2
    );
}

#[gpui::test]
fn update_waits_for_attaches_and_refuses_to_stop_when_saving_fails(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.begin_update(cx);
        model.pending.insert(PaneId::new());
        model.receive((1, Update::Flushed), cx);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate(_)))
    );
    view.update(cx, |model, cx| {
        model.pending.clear();
        model.path = model.path.join("invalid/state.json");
        model.receive((1, Update::Flushed), cx);
        assert!(model.quitting == Quitting::Idle);
        assert!(model.updates.ready.is_some());
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate(_)))
    );
}

#[gpui::test]
fn update_requires_connected_server_and_finished_settings(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.server_preferences.busy = true;
        model.begin_update(cx);
        assert!(model.quitting == Quitting::Idle);
        model.server_preferences.busy = false;
        model.disconnect(cx);
        model.begin_update(cx);
        assert!(model.quitting == Quitting::Idle);
        assert!(model.updates.ready.is_some());
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Flush | Work::PrepareUpdate(_)))
    );
}
