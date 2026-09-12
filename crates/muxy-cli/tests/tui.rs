#[path = "tui/fixture.rs"]
mod fixture;
mod support;

use fixture::{Fixture, Result, Tui};

#[test]
fn keyboard_layout_restores_without_respawning_and_detach_preserves_sessions() -> Result {
    let fixture = Fixture::new()?;
    let mut tui = Tui::start(&fixture, &[])?;
    tui.ready()?;
    tui.write(b"printf '\\nTUI_FIRST_READY\\n'\r")?;
    tui.output("TUI_FIRST_READY")?;
    tui.write(b"\x02c")?;
    tui.wait(|tui| Ok(tui.tabs()?.len() == 2))?;
    tui.ready()?;
    tui.write(b"\x02%")?;
    tui.wait(|tui| {
        Ok(tui.active_tab()?["panes"]
            .as_object()
            .is_some_and(|panes| panes.len() == 2))
    })?;
    tui.ready()?;
    let right = tui.active_tab()?["focus"].clone();
    tui.write(b"\x02\x1b[D")?;
    tui.wait(|tui| Ok(tui.active_tab()?["focus"] != right))?;
    tui.write(b"\x02\x1b[1;5C")?;
    tui.wait(|tui| {
        Ok(tui.active_tab()?["layout"]["Split"]["ratio"]
            .as_f64()
            .is_some_and(|ratio| ratio > 0.5))
    })?;
    tui.write(b"\x02z")?;
    tui.wait(|tui| Ok(tui.active_tab()?["zoom"] == true))?;
    tui.detach()?;
    let state = fixture.state()?;
    let client = fixture.client()?;
    let ids: Vec<_> = client
        .list_sessions()?
        .iter()
        .map(|session| session.id)
        .collect();
    assert_eq!(ids.len(), 3);
    let mut restored = Tui::start(&fixture, &[])?;
    restored.ready()?;
    assert_eq!(fixture.state()?, state);
    assert_eq!(
        client
            .list_sessions()?
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        ids
    );
    restored.detach()?;
    assert_eq!(client.list_sessions()?.len(), 3);
    Ok(())
}

#[test]
fn a_second_tui_and_redirected_invocations_leave_the_live_layout_unchanged() -> Result {
    let fixture = Fixture::new()?;
    let mut first = Tui::start(&fixture, &[])?;
    first.ready()?;
    let state = fixture.state()?;
    let sessions = fixture.client()?.list_sessions()?;
    let mut second = Tui::start(&fixture, &[])?;
    let status = second.exit()?;
    assert_eq!(status.code, Some(1));
    assert!(String::from_utf8_lossy(&second.raw).contains("another TUI"));
    let output = fixture.command().output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("requires terminal stdin and stdout"));
    assert_eq!(fixture.state()?, state);
    assert_eq!(fixture.client()?.list_sessions()?, sessions);
    first.detach()?;
    Ok(())
}

#[test]
fn close_confirms_a_foreground_program_and_does_not_confirm_background_work() -> Result {
    let fixture = Fixture::new()?;
    let mut tui = Tui::start(&fixture, &[])?;
    tui.ready()?;
    let client = fixture.client()?;
    let session = client.list_sessions()?[0].id;
    tui.write(b"stty -echo; printf '\\nCLOSE_READY\\n'; cat\r")?;
    tui.output("CLOSE_READY")?;
    tui.wait(|_| {
        let attached = client.attach(session, muxy_protocol::Size { cols: 80, rows: 24 })?;
        let foreground = attached
            .process
            .is_some_and(|process| !process.is_shell && process.name == "cat");
        client.detach(attached.channel)?;
        Ok(foreground)
    })?;
    tui.write(b"\x02x")?;
    tui.output("Close terminal?")?;
    assert_eq!(client.list_sessions()?.len(), 1);
    tui.write(b"\x1b")?;
    tui.write(b"\x02x\r")?;
    tui.wait(|tui| Ok(tui.tabs()?.is_empty() && client.list_sessions()?.is_empty()))?;
    tui.write(b"\x02c")?;
    tui.ready()?;
    tui.write(b"sleep 30 &\nprintf '\\nBACKGROUND_READY\\n'\r")?;
    tui.output("BACKGROUND_READY")?;
    tui.write(b"\x02x")?;
    tui.wait(|tui| Ok(tui.tabs()?.is_empty() && client.list_sessions()?.is_empty()))?;
    tui.detach()?;
    let mut restored = Tui::start(&fixture, &[])?;
    restored.output("No tabs.")?;
    assert!(client.list_sessions()?.is_empty());
    restored.detach()?;
    Ok(())
}

#[test]
fn shell_identity_is_replaced_and_the_hosting_session_cannot_attach_to_itself() -> Result {
    let fixture = Fixture::new()?;
    let mut tui = Tui::start(
        &fixture,
        &[("MUXY_SERVER_ID", "inherited"), ("MUXY_SESSION_ID", "999")],
    )?;
    tui.ready()?;
    let client = fixture.client()?;
    let catalog = client.catalog()?;
    let session = client.list_sessions()?[0].id;
    tui.write(b"printf '\\nSTAMP:%s:%s\\n' \"$MUXY_SERVER_ID\" \"$MUXY_SESSION_ID\"\r")?;
    tui.output(&format!("STAMP:{}:{}", catalog.server, session.get()))?;
    tui.detach()?;
    let server = catalog.server.to_string();
    let id = session.get().to_string();
    let mut nested = Tui::start(
        &fixture,
        &[("MUXY_SERVER_ID", &server), ("MUXY_SESSION_ID", &id)],
    )?;
    nested.output("Hosting terminal excluded")?;
    nested.write(b"\x02x")?;
    nested.output("Cannot close the terminal hosting this TUI")?;
    assert_eq!(client.list_sessions()?.len(), 1);
    nested.write(b"\x02c")?;
    nested.ready()?;
    nested.detach()?;
    assert_eq!(client.list_sessions()?.len(), 2);
    Ok(())
}
