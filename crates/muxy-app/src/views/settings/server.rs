use super::{Category, Change, SettingsEvent, SettingsPane};
use gpui::{AnyElement, Context};
use muxy_ui::controls;

pub(super) fn rows(pane: &SettingsPane, cx: &mut Context<SettingsPane>) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    let connected = pane.snapshot.connected;
    if pane.matches(Category::Server, "Current device connection") {
        rows.push(pane.row(
            "server",
            "Current device",
            controls::button(
                pane.style(),
                "server-read",
                if connected {
                    "Reload settings"
                } else {
                    "Connect"
                },
                !pane.snapshot.server_busy,
                cx.listener(move |_, _, _, cx| {
                    cx.emit(if connected {
                        SettingsEvent::ReadServer
                    } else {
                        SettingsEvent::Connect
                    });
                }),
            ),
        ));
    }
    if !connected {
        if pane.matches(Category::Server, "Server disconnected") {
            rows.push(pane.note(
                "Server disconnected. App preferences still work. Connect to edit server settings.",
                false,
            ));
        }
        return rows;
    }
    if let Some(server) = &pane.snapshot.server {
        for (id, label) in [
            ("default-shell", "Default shell (executable path)"),
            ("history-budget", "History budget per session (MiB)"),
        ] {
            if pane.matches(Category::Server, label) {
                rows.push(pane.row(id, label, pane.field(id)));
            }
        }
        if pane.matches(Category::Server, "Shell integration") {
            rows.push(pane.row(
                "shell-integration",
                "Shell integration",
                pane.toggle(
                    "shell-integration",
                    server.shell_integration,
                    Change::ShellIntegration(!server.shell_integration),
                    cx,
                ),
            ));
        }
        if pane.matches(
            Category::Server,
            "Default shell history budget shell integration",
        ) {
            rows.push(pane.note("These settings apply to new sessions. The saved-history budget changes at the next server start. An empty shell uses $SHELL, or /bin/zsh.", false));
        }
    } else if pane.matches(Category::Server, "Loading settings") {
        rows.push(pane.note("Load the current device's settings to edit them.", false));
    }
    for (restart, label) in [(false, "Stop Server"), (true, "Restart Server")] {
        if pane.matches(Category::Server, label) {
            rows.push(pane.row(
                label,
                label,
                controls::button(
                    pane.style(),
                    label,
                    label,
                    !pane.snapshot.server_busy,
                    cx.listener(move |_, _, _, cx| {
                        cx.emit(SettingsEvent::ServerControl { restart });
                    }),
                ),
            ));
        }
    }
    rows
}
