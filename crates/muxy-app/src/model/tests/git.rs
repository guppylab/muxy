use super::projects::two_projects;
use super::*;
use muxy_protocol::{GitAction, GitReply, GitRequest, GitSummary};

#[gpui::test]
fn git_results_stay_with_the_requested_project_after_switching(cx: &mut TestAppContext) {
    let (state, _, _, _, _) = two_projects();
    let first = state.current_project().id;
    let second = state
        .projects()
        .iter()
        .find(|p| p.id != first && !p.home)
        .expect("second")
        .id;
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        requests.try_iter().for_each(drop);
        model.git_request(first, GitAction::Summary, cx);
        model.select_project(second, cx);
        model.receive_git(
            &GitRequest {
                project: first,
                action: GitAction::Summary,
            },
            Ok(GitReply::Summary(Some(GitSummary {
                branch: Some("first-branch".into()),
                ..GitSummary::default()
            }))),
            cx,
        );
        assert_eq!(model.state.current_project().id, second);
        assert_eq!(
            model.git.projects[&first]
                .summary
                .as_ref()
                .expect("summary")
                .branch
                .as_deref(),
            Some("first-branch")
        );
        assert!(
            model
                .git
                .projects
                .get(&second)
                .is_none_or(|r| r.summary.is_none())
        );
    });
}

#[gpui::test]
fn git_refreshes_coalesce_and_disconnected_mutations_are_not_queued(cx: &mut TestAppContext) {
    let (state, _, _, _, _) = two_projects();
    let project = state.current_project().id;
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        requests.try_iter().for_each(drop);
        for _ in 0..4 {
            model.git_request(project, GitAction::Summary, cx);
        }
        assert_eq!(
            requests
                .try_iter()
                .filter(|(_, work)| matches!(work, Work::Git(_)))
                .count(),
            1
        );
        model.disconnect(cx);
        requests.try_iter().for_each(drop);
        model.git_request(project, GitAction::DeleteBranch("branch".into()), cx);
        assert!(
            !requests
                .try_iter()
                .any(|(_, work)| matches!(work, Work::Git(_)))
        );
    });
}

#[gpui::test]
fn an_accepted_git_action_waits_for_background_refresh(cx: &mut TestAppContext) {
    let (state, project, _, _, _) = two_projects();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        requests.try_iter().for_each(drop);
        model.git_request(project, GitAction::Summary, cx);
        model.git_request(project, GitAction::DeleteBranch("confirmed".into()), cx);
        assert_eq!(requests.try_iter().filter(|(_, work)| matches!(work, Work::Git(_))).count(), 1);
        model.receive_git(&GitRequest { project, action: GitAction::Summary }, Ok(GitReply::Summary(None)), cx);
        assert!(requests.try_iter().any(|(_, work)| matches!(work, Work::Git(request) if request.action == GitAction::DeleteBranch("confirmed".into()))));
    });
}

#[gpui::test]
fn stale_git_mutations_and_inspections_do_not_change_a_new_form(cx: &mut TestAppContext) {
    let (state, first, second, _, _) = two_projects();
    let (boot, _) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.git_request(first, GitAction::CreateBranch("old".into()), cx);
        model.select_project(second, cx);
        model.open_git_form(second, false, cx);
        model.receive_git(
            &GitRequest {
                project: first,
                action: GitAction::CreateBranch("old".into()),
            },
            Ok(GitReply::Done),
            cx,
        );
        assert!(matches!(model.overlay, Some(Overlay::GitForm(_))));
        model.git_request(first, GitAction::InspectRemoval, cx);
        model.open_git_form(second, false, cx);
        let expected = muxy_protocol::WorktreeRemoval {
            directory: muxy_protocol::ServerPath(b"/unused".to_vec()),
            device: 1,
            inode: 1,
            dirty: false,
            status: vec![],
            head: None,
            branch: None,
        };
        model.receive_git(
            &GitRequest {
                project: first,
                action: GitAction::InspectRemoval,
            },
            Ok(GitReply::Removal(expected)),
            cx,
        );
        assert!(matches!(model.overlay, Some(Overlay::GitForm(_))));
        assert!(model.close_prompt.is_none());
    });
}

#[gpui::test]
fn git_popovers_open_from_their_controls_and_follow_their_anchors(cx: &mut TestAppContext) {
    let (state, _, _, _, _) = two_projects();
    let project = state.current_project().id;
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Summary,
            },
            Ok(GitReply::Summary(Some(GitSummary {
                branch: Some("feature".into()),
                changed: 2,
                ..GitSummary::default()
            }))),
            cx,
        );
    });
    cx.simulate_resize(size(px(1200.0), px(800.0)));
    cx.run_until_parked();
    let changes = cx.debug_bounds("git-changes-status").expect("changes");
    cx.simulate_click(changes.center(), Modifiers::none());
    cx.run_until_parked();
    view.read_with(cx, |model, _| {
        assert!(
            model.git.changes_anchor.get().is_some(),
            "changes anchor was measured"
        );
        assert!(
            matches!(model.overlay, Some(Overlay::Git(_))),
            "changes picker is open"
        );
    });
    let panel = cx.debug_bounds("git-picker").expect("changes popover");
    assert!(panel.bottom() <= changes.top());
    cx.simulate_resize(size(px(700.0), px(500.0)));
    cx.run_until_parked();
    let panel = cx.debug_bounds("git-picker").expect("resized popover");
    let changes = cx
        .debug_bounds("git-changes-status")
        .expect("resized changes");
    assert!(panel.bottom() <= changes.top());
    assert!(panel.left() >= px(0.0) && panel.right() <= px(700.0));
    assert!(panel.left() <= changes.right() && panel.right() >= changes.left());
}

#[gpui::test]
fn git_form_fits_within_small_windows(cx: &mut TestAppContext) {
    let (state, project, _, _, _) = two_projects();
    let (boot, _) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| model.open_git_form(project, true, cx));
    for viewport in [size(px(1200.0), px(900.0)), size(px(600.0), px(400.0))] {
        cx.simulate_resize(viewport);
        cx.run_until_parked();
        let form = cx.debug_bounds("git-form").expect("form");
        assert!(form.top() >= px(0.0) && form.bottom() <= viewport.height);
        assert!(form.left() >= px(0.0) && form.right() <= viewport.width);
    }
}

#[gpui::test]
fn distinct_reads_are_retained_and_cached_controls_stay_usable(cx: &mut TestAppContext) {
    let (state, project, _, _, _) = two_projects();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        requests.try_iter().for_each(drop);
        model.git_request(project, GitAction::Branches, cx);
        model.git_request(project, GitAction::Changes, cx);
        assert!(!model.git.projects[&project].busy());
        requests.try_iter().for_each(drop);
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Branches,
            },
            Ok(GitReply::Branches(vec![])),
            cx,
        );
        assert!(requests.try_iter().any(
            |(_, work)| matches!(work, Work::Git(request) if request.action == GitAction::Changes)
        ));
        assert!(!model.git.projects[&project].busy());
    });
}

#[gpui::test]
fn background_git_errors_do_not_raise_alerts_or_retry_on_idle(cx: &mut TestAppContext) {
    let (state, project, _, _, _) = two_projects();
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.git_request(project, GitAction::Summary, cx);
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Summary,
            },
            Err(muxy_client::ClientError::Disconnected),
            cx,
        );
        assert!(model.error.is_none());
    });
    requests.try_iter().for_each(drop);
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert!(
        !requests
            .try_iter()
            .any(|(_, work)| matches!(work, Work::Git(_)))
    );
}

#[gpui::test]
fn filesystem_invalidations_refresh_only_the_active_project(cx: &mut TestAppContext) {
    let (state, _, _, _, _) = two_projects();
    let project = state.current_project().id;
    let (boot, requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        model.git.reset_context();
        model.sync_git(cx);
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Watch,
            },
            Ok(GitReply::Done),
            cx,
        );
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Summary,
            },
            Ok(GitReply::Summary(Some(GitSummary {
                branch: Some("main".into()),
                ..GitSummary::default()
            }))),
            cx,
        );
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Branches,
            },
            Ok(GitReply::Branches(vec![])),
            cx,
        );
        requests.try_iter().for_each(drop);
        model.git_invalidated(ProjectId::new(), cx);
        assert!(requests.try_iter().next().is_none());
        model.git_invalidated(project, cx);
        assert!(requests.try_iter().any(
            |(_, work)| matches!(work, Work::Git(request) if request.action == GitAction::Summary)
        ));
        assert!(!model.git.projects[&project].busy());
    });
}

#[gpui::test]
fn successful_refresh_recovers_a_rejected_branch_switch(cx: &mut TestAppContext) {
    let (state, project, _, _, _) = two_projects();
    let (boot, _requests) = stub_boot(state);
    let (view, cx) = cx.add_window_view(|window, cx| AppModel::new(boot, window, cx));
    view.update(cx, |model, cx| {
        model.connection = ConnectionState::Ready;
        let request = GitRequest {
            project,
            action: GitAction::SwitchBranch("busy-branch".into()),
        };
        model.git_request(project, request.action.clone(), cx);
        model.receive_git(
            &request,
            Err(muxy_client::ClientError::Invalid(
                muxy_protocol::ErrorCode::BadRequest,
            )),
            cx,
        );
        assert!(
            model.git.projects[&project]
                .load_error(&GitAction::Branches)
                .is_some()
        );
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Summary,
            },
            Ok(GitReply::Summary(Some(GitSummary {
                branch: Some("main".into()),
                ..GitSummary::default()
            }))),
            cx,
        );
        model.git_request(project, GitAction::Branches, cx);
        model.receive_git(
            &GitRequest {
                project,
                action: GitAction::Branches,
            },
            Ok(GitReply::Branches(vec![muxy_protocol::GitBranch {
                name: "main".into(),
                current: true,
                checked_out: true,
                default: true,
            }])),
            cx,
        );
        let repository = &model.git.projects[&project];
        assert!(repository.load_error(&GitAction::Branches).is_none());
        assert!(repository.has_loaded(&GitAction::Branches));
        assert!(!repository.busy());
        assert_eq!(repository.branches[0].name, "main");
    });
}
