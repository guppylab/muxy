use super::*;

fn ready(model: &mut AppModel) {
    model.connection = ConnectionState::Ready;
    model.updates.server = Some(muxy_protocol::ServerInfo::current());
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
    cx.simulate_prompt_answer("Not Now");
    cx.run_until_parked();
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. } | Work::Flush))
    );
    view.update(cx, |model, cx| model.check_for_updates(true, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update and Restart App");
    cx.run_until_parked();
    let work: Vec<_> = requests.try_iter().collect();
    assert!(work.iter().any(|(_, work)| matches!(work, Work::Flush)));
    assert!(
        !work
            .iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. }))
    );
    view.update(cx, |model, cx| model.receive((1, Update::Flushed), cx));
    assert!(
        requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. }))
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
                Update::PreparedForInstall(Err(muxy_client::ClientError::Timeout)),
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
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. }))
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
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. }))
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
            .any(|(_, work)| matches!(work, Work::Flush | Work::PrepareUpdate { .. }))
    );
}

#[gpui::test]
fn incompatible_update_waits_and_can_be_cancelled_without_shutdown(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model
            .updates
            .ready
            .as_mut()
            .expect("update")
            .build
            .as_mut()
            .expect("build")
            .compatibility += 1;
        model.updates.sessions = 2;
        model.check_for_updates(true, cx);
    });
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update When Sessions End");
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(model.updates.scheduled);
        assert!(
            model
                .update_status()
                .expect("status")
                .contains("2 terminal sessions")
        );
        let record: serde_json::Value = serde_json::from_slice(
            &std::fs::read(model.path.with_file_name("pending-update.json")).expect("record"),
        )
        .expect("json");
        assert_eq!(record["scheduled"], true);
        assert_eq!(record["version"], "2.0.0-beta-1234");
        assert!(model.quitting == Quitting::Idle);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. } | Work::Flush))
    );
    view.update(cx, |model, cx| model.check_for_updates(true, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Cancel Scheduled Update");
    cx.run_until_parked();
    view.read_with(cx, |model, _| assert!(!model.updates.scheduled));
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. } | Work::Flush))
    );
}

#[gpui::test]
fn ending_sessions_requires_a_second_explicit_confirmation(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.updates.ready.as_mut().expect("update").build = None;
        model.check_for_updates(true, cx);
    });
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update and End Sessions…");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Flush))
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    view.read_with(cx, |model, _| assert!(model.quitting == Quitting::Idle));
    view.update(cx, |model, cx| model.check_for_updates(true, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update and End Sessions…");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Update and End Sessions");
    cx.run_until_parked();
    view.update(cx, |model, cx| model.receive((1, Update::Flushed), cx));
    assert!(requests.try_iter().any(|(_, work)| matches!(
        work,
        Work::PrepareUpdate {
            mode: crate::server::UpdateMode::EndSessions,
            ..
        }
    )));
}

#[gpui::test]
fn scheduled_install_uses_atomic_idle_check_and_resumes_waiting_if_busy(cx: &mut TestAppContext) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.updates.scheduled = true;
        let status = crate::server::ServerUpdate {
            server: model.updates.server.clone().expect("server"),
            sessions: 0,
            replaced: false,
        };
        model.receive((1, Update::ServerChecked(Ok(status))), cx);
        assert!(model.quitting == Quitting::Update);
        model.receive((1, Update::Flushed), cx);
    });
    assert!(requests.try_iter().any(|(_, work)| matches!(
        work,
        Work::PrepareUpdate {
            mode: crate::server::UpdateMode::WhenIdle,
            ..
        }
    )));
    view.update(cx, |model, cx| {
        model.receive((1, Update::PreparedForInstall(Ok(None))), cx);
        assert!(model.quitting == Quitting::Idle);
        assert!(model.updates.scheduled);
        assert!(model.updates.ready.is_some());
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::PrepareUpdate { .. }))
    );
}

#[gpui::test]
fn only_update_notifications_reconnect_other_clients_and_queue_new_terminals(
    cx: &mut TestAppContext,
) {
    let mut state = AppState::bootstrap().expect("state");
    state.open_terminal_tab(state.home().id).expect("tab");
    let pane = state.home().tabs[0].panes[0].id;
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.receive((1, Update::Event(ClientEvent::Disconnected)), cx);
        assert!(model.connection == ConnectionState::Disconnected);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Connect | Work::ReconnectAfterUpdate(_)))
    );
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.receive((1, Update::Event(ClientEvent::ServerRestarting)), cx);
        model.start_attach(pane, Size { cols: 80, rows: 24 }, cx);
        assert!(model.updates.queued_attaches.contains_key(&pane));
        model.receive((1, Update::Event(ClientEvent::Disconnected)), cx);
        assert!(model.connection == ConnectionState::Connecting);
    });
    assert!(
        requests
            .try_iter()
            .any(|(generation, work)| generation == 2
                && matches!(work, Work::ReconnectAfterUpdate(_)))
    );
    view.update(cx, |model, cx| {
        model.receive((2, Update::ConnectFailed("start failed".into())), cx);
        assert!(model.connection == ConnectionState::Disconnected);
        assert!(model.update_status().expect("status").contains("Retry"));
        model.reconcile_server_update(cx);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Connect | Work::ReconnectAfterUpdate(_)))
    );
}

#[gpui::test]
fn explicit_stop_does_not_reconnect_when_another_client_announces_an_update(
    cx: &mut TestAppContext,
) {
    let (boot, requests) = stub_boot(AppState::bootstrap().expect("state"));
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        ready(model);
        model.server_preferences.control_busy = true;
        model.receive((1, Update::Event(ClientEvent::ServerRestarting)), cx);
        model.receive((1, Update::Event(ClientEvent::Disconnected)), cx);
        assert!(model.connection == ConnectionState::Disconnected);
    });
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Connect | Work::ReconnectAfterUpdate(_)))
    );
}
