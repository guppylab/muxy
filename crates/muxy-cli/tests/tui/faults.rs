use std::fs;

use super::fixture::{Fixture, Result, Tui};
use super::proxy::{Point, Proxy};

#[test]
fn first_launch_crashes_before_creation_or_its_reply_reuse_the_durable_token() -> Result {
    for point in [Point::CreateRequest, Point::CreateReply] {
        let fixture = Fixture::new()?;
        let proxy = Proxy::new(&fixture)?;
        proxy.arm(point);
        let mut tui = Tui::start(&fixture, &[])?;
        tui.wait(|_| Ok(proxy.reached()))?;
        let before = tui.active_tab()?;
        let pane = before["panes"]
            .as_object()
            .ok_or("panes")?
            .values()
            .next()
            .ok_or("pane")?;
        assert!(!pane["creation"].is_null());
        assert!(pane["session"].is_null());
        tui.signal("KILL")?;
        tui.exit()?;
        proxy.disconnect();
        let mut restored = Tui::start(&fixture, &[])?;
        restored.ready()?;
        let client = fixture.client()?;
        assert_eq!(client.list_sessions()?.len(), 1);
        assert_eq!(restored.active_tab()?["focus"], before["focus"]);
        let saved = fixture.state()?;
        restored.detach()?;
        let mut again = Tui::start(&fixture, &[])?;
        again.ready()?;
        assert_eq!(fixture.state()?, saved);
        assert_eq!(client.list_sessions()?.len(), 1);
        again.detach()?;
    }
    Ok(())
}

#[test]
fn close_intents_survive_crashes_and_disconnects_on_both_sides_of_the_reply() -> Result {
    for point in [Point::DiscardRequest, Point::DiscardReply] {
        for crash in [false, true] {
            let fixture = Fixture::new()?;
            let proxy = Proxy::new(&fixture)?;
            let mut tui = Tui::start(&fixture, &[])?;
            tui.ready()?;
            proxy.arm(point);
            tui.write(b"\x02x")?;
            tui.wait(|_| Ok(proxy.reached()))?;
            assert!(tui.tabs()?.is_empty());
            assert_eq!(
                fixture.state()?["discards"]
                    .as_array()
                    .ok_or("discards")?
                    .len(),
                1
            );
            if crash {
                tui.signal("KILL")?;
                tui.exit()?;
            }
            proxy.disconnect();
            if crash {
                tui = Tui::start(&fixture, &[])?;
            }
            tui.output("No tabs")?;
            tui.wait(|_| {
                Ok(fixture.state()?["discards"]
                    .as_array()
                    .is_some_and(Vec::is_empty))
            })?;
            assert!(fixture.client()?.list_sessions()?.is_empty());
            tui.detach()?;
            let mut again = Tui::start(&fixture, &[])?;
            again.output("No tabs")?;
            assert!(fixture.client()?.list_sessions()?.is_empty());
            again.detach()?;
        }
    }
    Ok(())
}

#[test]
fn failed_close_save_sends_no_discard_and_failed_ack_save_replays_after_repair() -> Result {
    let fixture = Fixture::new()?;
    let proxy = Proxy::new(&fixture)?;
    let mut tui = Tui::start(&fixture, &[])?;
    tui.ready()?;
    let before = fixture.state()?;
    let state = fixture.directory.path().join("tui-state.json");
    let backup = fixture.directory.path().join("state-backup");
    let block = || -> Result {
        fs::rename(&state, &backup)?;
        fs::create_dir(&state)?;
        Ok(())
    };
    let repair = || -> Result {
        fs::remove_dir(&state)?;
        fs::rename(&backup, &state)?;
        Ok(())
    };
    block()?;
    tui.write(b"\x02x")?;
    assert_eq!(tui.exit()?.code, Some(1));
    tui.assert_restored()?;
    assert_eq!(fixture.client()?.list_sessions()?.len(), 1);
    repair()?;
    assert_eq!(fixture.state()?, before);
    let mut tui = Tui::start(&fixture, &[])?;
    tui.ready()?;
    proxy.arm(Point::DiscardReply);
    tui.write(b"\x02x")?;
    tui.wait(|_| Ok(proxy.reached()))?;
    assert!(fixture.client()?.list_sessions()?.is_empty());
    block()?;
    proxy.release();
    assert_eq!(tui.exit()?.code, Some(1));
    tui.assert_restored()?;
    repair()?;
    assert_eq!(
        fixture.state()?["discards"]
            .as_array()
            .ok_or("discards")?
            .len(),
        1
    );
    let mut restored = Tui::start(&fixture, &[])?;
    restored.output("No tabs")?;
    restored.wait(|_| {
        Ok(fixture.state()?["discards"]
            .as_array()
            .is_some_and(Vec::is_empty))
    })?;
    assert!(fixture.client()?.list_sessions()?.is_empty());
    restored.detach()?;
    Ok(())
}

#[test]
fn typing_and_frame_acknowledgements_continue_while_a_picker_request_waits() -> Result {
    let fixture = Fixture::new()?;
    let proxy = Proxy::new(&fixture)?;
    let mut tui = Tui::start(&fixture, &[])?;
    tui.ready()?;
    proxy.arm(Point::ListReply);
    tui.write(b"\x02w")?;
    tui.wait(|_| Ok(proxy.reached()))?;
    tui.write(b"\x1b")?;
    tui.wait(|tui| {
        Ok(!tui
            .text()?
            .iter()
            .any(|row| row.contains("Existing terminals")))
    })?;
    tui.write(b"i=0; while [ $i -lt 100 ]; do echo frame-$i; i=$((i+1)); sleep 0.01; done; echo RESPONSIVE_DONE\r")?;
    tui.output("RESPONSIVE_DONE")?;
    proxy.release();
    tui.detach()?;
    Ok(())
}
