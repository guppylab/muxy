use super::overlays::Overlay;
use crate::{boot::Work, model::AppModel};
use gpui::{AppContext, Context, Focusable, Window};
use muxy_app_core::ProjectId;
use muxy_protocol::{ProjectSession, ProjectSessions, SessionId, SessionStatus};
use muxy_ui::command_popover::{
    CommandPopover, CommandPopoverAction, CommandPopoverConfig, CommandPopoverDensity,
    CommandPopoverEvent, CommandPopoverItem, CommandPopoverPresentation, CommandPopoverRow,
    CommandPopoverStatus, CommandPopoverTab,
};

pub(crate) struct SessionPicker {
    pub(crate) project: ProjectId,
    pub(crate) picker: gpui::Entity<CommandPopover>,
    entries: Vec<ProjectSession>,
    after: Option<SessionId>,
    revision: Option<u64>,
    next: Option<SessionId>,
}

impl AppModel {
    pub(crate) fn open_session_picker(
        &mut self,
        project: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = cx.new(|cx| {
            CommandPopover::new(
                CommandPopoverConfig {
                    id: "project-terminals".into(),
                    presentation: CommandPopoverPresentation::Modal,
                    density: CommandPopoverDensity::Comfortable,
                    tabs: vec![CommandPopoverTab::new("sessions", "Existing Terminals")],
                    placeholder: "Filter this page…".into(),
                    footer_actions: Vec::new(),
                    footer_hints: Vec::new(),
                    width: Some(640.0),
                    height: Some(460.0),
                    max_height: None,
                    completion_on_tab: false,
                    confirm_on_click: true,
                },
                self.theme.clone(),
                self.metrics,
                cx,
            )
        });
        picker.focus_handle(cx).focus(window);
        self.overlay_subscription =
            Some(cx.subscribe(&picker, |model, _, event, cx| match event {
                CommandPopoverEvent::Confirmed(selection) => {
                    model.choose_existing_session(selection.id.as_ref(), cx);
                }
                CommandPopoverEvent::Dismissed => model.dismiss_overlay(cx),
                CommandPopoverEvent::QueryChanged { query, .. } => model.filter_sessions(query, cx),
                CommandPopoverEvent::FooterAction(action) if action == "next" => {
                    model.next_session_page(cx);
                }
                _ => {}
            }));
        self.overlay = Some(Overlay::Sessions(SessionPicker {
            project,
            picker,
            entries: Vec::new(),
            after: None,
            revision: None,
            next: None,
        }));
        self.refresh_session_picker(cx);
    }

    pub(crate) fn refresh_session_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(Overlay::Sessions(picker)) = &mut self.overlay {
            picker.after = None;
            let project = picker.project;
            self.request_session_page(project, None, None, cx);
        }
    }

    fn next_session_page(&mut self, cx: &mut Context<Self>) {
        if let Some(Overlay::Sessions(picker)) = &mut self.overlay
            && let Some(after) = picker.next
        {
            let project = picker.project;
            let revision = picker.revision;
            picker.after = Some(after);
            self.request_session_page(project, Some(after), revision, cx);
        }
    }

    pub(crate) fn receive_session_page(
        &mut self,
        project: ProjectId,
        after: Option<SessionId>,
        result: Result<ProjectSessions, muxy_client::ClientError>,
        cx: &mut Context<Self>,
    ) {
        let Some(Overlay::Sessions(picker)) = &mut self.overlay else {
            return;
        };
        if picker.project != project || picker.after != after {
            return;
        }
        match result {
            Ok(page) => {
                picker.entries = page
                    .sessions
                    .into_iter()
                    .filter(|session| {
                        matches!(
                            session.status,
                            SessionStatus::Live | SessionStatus::Starting
                        )
                    })
                    .collect();
                picker.next = page.next;
                picker.revision = Some(page.revision);
                let actions = if picker.next.is_some() {
                    vec![CommandPopoverAction::new("next", "Next Page")]
                } else {
                    Vec::new()
                };
                picker
                    .picker
                    .update(cx, |picker, cx| picker.set_footer_actions(actions, cx));
                self.filter_sessions("", cx);
            }
            Err(muxy_client::ClientError::Server(error))
                if error.code == muxy_protocol::ErrorCode::CatalogChanged =>
            {
                picker.after = None;
                self.request_session_page(project, None, None, cx);
            }
            Err(error) => picker.picker.update(cx, |picker, cx| {
                picker.set_status(CommandPopoverStatus::Error(error.to_string().into()), cx);
            }),
        }
    }

    fn request_session_page(
        &mut self,
        project: ProjectId,
        after: Option<SessionId>,
        revision: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        self.send_session_request(
            Work::ProjectSessions {
                project,
                after,
                revision,
            },
            cx,
        );
    }

    fn filter_sessions(&self, query: &str, cx: &mut Context<Self>) {
        let Some(Overlay::Sessions(picker)) = &self.overlay else {
            return;
        };
        let query = query.to_lowercase();
        let items: Vec<_> = picker
            .entries
            .iter()
            .filter_map(|session| {
                let status = match session.status {
                    SessionStatus::Live => "Running",
                    SessionStatus::Starting => "Starting",
                    SessionStatus::Ended => "Ended",
                    SessionStatus::Unavailable => "Output unavailable",
                };
                let label = format!(
                    "{} · {status} · {}",
                    session.info.id.get(),
                    String::from_utf8_lossy(&session.info.directory.0)
                );
                label.to_lowercase().contains(&query).then(|| {
                    CommandPopoverItem::Row(CommandPopoverRow::new(
                        session.info.id.get().to_string(),
                        label,
                    ))
                })
            })
            .collect();
        let status = if items.is_empty() {
            CommandPopoverStatus::Empty("No terminals on this page".into())
        } else {
            CommandPopoverStatus::Ready
        };
        picker.picker.update(cx, |picker, cx| {
            picker.set_items(items, cx);
            picker.set_status(status, cx);
        });
    }

    fn choose_existing_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(Overlay::Sessions(picker)) = &self.overlay else {
            return;
        };
        let Some(session) = id
            .parse()
            .ok()
            .and_then(SessionId::new)
            .and_then(|id| picker.entries.iter().find(|entry| entry.info.id == id))
            .cloned()
        else {
            return;
        };
        let project = picker.project;
        self.open_existing_session(project, &session, cx);
    }
}
