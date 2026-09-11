use std::time::Duration;

use gpui::{Context, Task};

use super::{AppModel, ConnectionState, Quitting};
use crate::boot::Work;
use crate::updater::{Installation, PreparedUpdate};

#[derive(Default)]
pub(super) struct Updater {
    installation: Option<Installation>,
    unavailable: Option<String>,
    checking: bool,
    manual: bool,
    pub(super) ready: Option<PreparedUpdate>,
    task: Option<Task<()>>,
    poll: Option<Task<()>>,
    previous_bundle: Option<tempfile::TempDir>,
}

impl AppModel {
    pub(super) fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        if crate::updater::build_number(env!("CARGO_PKG_VERSION")).is_none() {
            self.updates.unavailable =
                Some("Automatic updates are available in installed releases of Muxy Beta".into());
            return;
        }
        let detect = cx
            .background_executor()
            .spawn(async { Installation::detect() });
        self.updates.task = Some(cx.spawn(async move |model, cx| {
            let result = detect.await;
            let _ = model.update(cx, |model, cx| match result {
                Ok(installation) => {
                    model.updates.installation = Some(installation);
                    model.check_for_updates(false, cx);
                    model.updates.poll = Some(cx.spawn(async move |model, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_secs(3600))
                                .await;
                            if model
                                .update(cx, |model, cx| model.check_for_updates(false, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }));
                }
                Err(error) => model.updates.unavailable = Some(error.to_string()),
            });
        }));
    }

    pub(crate) fn update_status(&self) -> Option<&'static str> {
        if self.quitting == Quitting::Update {
            Some("Installing update…")
        } else if self.updates.ready.is_some() {
            Some("Restart to Update…")
        } else if self.updates.checking {
            Some("Checking for updates…")
        } else {
            None
        }
    }

    pub(crate) fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.quitting != Quitting::Idle {
            return;
        }
        if self.updates.ready.is_some() {
            if manual {
                self.confirm_update(cx);
            }
            return;
        }
        self.updates.manual |= manual;
        if self.updates.checking {
            return;
        }
        let Some(installation) = self.updates.installation.clone() else {
            if manual {
                self.update_message(
                    self.updates
                        .unavailable
                        .as_deref()
                        .unwrap_or("The updater is starting. Try again shortly."),
                    cx,
                );
            }
            self.updates.manual = false;
            return;
        };
        self.updates.checking = true;
        let prepare = cx
            .background_executor()
            .spawn(async move { installation.prepare() });
        self.updates.task = Some(cx.spawn(async move |model, cx| {
            let result = prepare.await;
            let _ = model.update(cx, |model, cx| {
                model.updates.checking = false;
                let manual = std::mem::take(&mut model.updates.manual);
                match result {
                    Ok(update) => {
                        model.updates.ready = update;
                        if manual {
                            if model.updates.ready.is_some() {
                                model.confirm_update(cx);
                            } else {
                                model
                                    .update_message("You’re running the latest Muxy 2.x beta.", cx);
                            }
                        }
                    }
                    Err(error) if manual => {
                        model.update_message(&format!("Could not check for updates: {error}"), cx);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn update_message(&self, message: &str, cx: &mut Context<Self>) {
        let _ = self.window.update(cx, |_, window, cx| {
            let response = window.prompt(
                gpui::PromptLevel::Info,
                "Muxy Beta Updates",
                Some(message),
                &["OK"],
                cx,
            );
            cx.spawn(async move |_| {
                let _ = response.await;
            })
            .detach();
        });
    }

    fn confirm_update(&mut self, cx: &mut Context<Self>) {
        if self.close_prompt.is_some() || self.quitting != Quitting::Idle {
            return;
        }
        let Some(update) = &self.updates.ready else {
            return;
        };
        let version = update.version.clone();
        let window = self.window;
        self.close_prompt = Some(cx.spawn(async move |model, cx| {
            let response = crate::views::confirm::prompt_update(window, &version, cx).await;
            let _ = model.update(cx, |model, cx| {
                model.close_prompt = None;
                match response {
                    Ok(true) => model.begin_update(cx),
                    Ok(false) => {}
                    Err(error) => model.fail(error, cx),
                }
            });
        }));
    }

    pub(super) fn begin_update(&mut self, cx: &mut Context<Self>) {
        if self.quitting != Quitting::Idle
            || self.updates.ready.is_none()
            || !self.preferences_before_quit(cx)
        {
            return;
        }
        if self.connection != ConnectionState::Ready {
            self.fail("Connect to the server before installing the update so running sessions can be stopped safely".into(), cx);
            return;
        }
        self.quitting = Quitting::Update;
        if !self.send(Work::Flush, cx) {
            self.quitting = Quitting::Idle;
        }
        cx.notify();
    }

    pub(super) fn flush_before_update(&mut self, cx: &mut Context<Self>) {
        if !self.pending.is_empty() || !self.discarding.is_empty() {
            if !self.send(Work::Flush, cx) {
                self.quitting = Quitting::Idle;
            }
        } else if !self.save(cx)
            || !self.send(
                Work::PrepareUpdate(self.path.with_file_name("server.sock")),
                cx,
            )
        {
            self.quitting = Quitting::Idle;
        }
    }

    pub(super) fn receive_update_prepared(
        &mut self,
        result: Result<std::fs::File, muxy_client::ClientError>,
        cx: &mut Context<Self>,
    ) {
        if self.quitting != Quitting::Update {
            return;
        }
        let lock = match result {
            Ok(lock) => lock,
            Err(error) => {
                self.quitting = Quitting::Idle;
                self.fail(format!("Could not prepare the update: {error}"), cx);
                return;
            }
        };
        self.disconnect(cx);
        let Some(update) = self.updates.ready.take() else {
            self.quitting = Quitting::Idle;
            return;
        };
        let install = cx
            .background_executor()
            .spawn(async move { update.install(lock) });
        self.updates.task = Some(cx.spawn(async move |model, cx| {
            let result = install.await;
            let _ = model.update(cx, |model, cx| match result {
                Ok(backup) => {
                    model.updates.previous_bundle = Some(backup);
                    cx.quit();
                }
                Err(error) => {
                    model.quitting = Quitting::Idle;
                    model.fail(format!("Could not install the beta: {error}"), cx);
                }
            });
        }));
    }
}
