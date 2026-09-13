use super::*;
use serde_json::{Value, json};

fn project(id: ProjectId, home: bool, pane: ProjectId, session: SessionId) -> Value {
    json!({
        "id": id, "server_id": "9eedd63357b54359b76e762c6bc2775c", "home": home,
        "name": if home { "Home" } else { "Project" }, "icon": null, "color": "#808080",
        "directory": "/tmp", "kind": null, "parent_id": null,
        "tabs": [{"panes": [{"id": pane, "content": {"type": "terminal", "session": session}}]}]
    })
}

fn fixture() -> Value {
    json!({"version": 1, "projects": [
        project(ProjectId::from_u128(1), true, ProjectId::from_u128(11), SessionId::new(1).unwrap()),
        project(ProjectId::from_u128(2), false, ProjectId::from_u128(12), SessionId::new(2).unwrap())
    ], "quick_terminal": {"id": ProjectId::from_u128(13), "content": {"type": "terminal", "session": 3}},
       "pending_discards": [4]})
}

#[test]
fn imports_identity_duplicate_directories_quick_terminal_and_pending_discards() {
    let snapshot = fixture();
    let imported = parse(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(imported.projects.len(), 2);
    assert!(imported.projects[0].home);
    assert_eq!(
        imported.projects[0].directory,
        imported.projects[1].directory
    );
    for (session, project) in [(1, 1), (2, 2), (3, 1), (4, 1)] {
        assert_eq!(
            imported.sessions[&SessionId::new(session).unwrap()],
            ProjectId::from_u128(project)
        );
    }
}

#[test]
fn rejects_conflicting_owners_duplicate_panes_and_unsupported_server_or_version() {
    for (pointer, value, reason) in [
        (
            "/projects/1/tabs/0/panes/0/content/session",
            json!(1),
            "conflicting projects",
        ),
        (
            "/projects/1/tabs/0/panes/0/id",
            json!(ProjectId::from_u128(11)),
            "duplicate legacy pane",
        ),
        (
            "/projects/1/server_id",
            json!(ProjectId::new()),
            "unsupported server",
        ),
        ("/version", json!(2), "unsupported legacy"),
    ] {
        let mut snapshot = fixture();
        *snapshot.pointer_mut(pointer).unwrap() = value;
        let error = parse(&serde_json::to_vec(&snapshot).unwrap()).unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
    }
}

#[test]
fn committed_catalog_prevents_reading_or_reimporting_the_legacy_file() -> io::Result<()> {
    let profile = std::env::temp_dir().join(format!("muxy-import-{}", ProjectId::new()));
    std::fs::create_dir_all(profile.join("sessions"))?;
    let source = serde_json::to_vec(&fixture())?;
    std::fs::write(profile.join("state.json"), &source)?;
    assert_eq!(read(&profile)?.projects.len(), 2);
    assert_eq!(std::fs::read(profile.join("state.json"))?, source);
    std::fs::write(profile.join("sessions/catalog.json"), b"committed receipt")?;
    std::fs::write(profile.join("state.json"), b"not parsed after import")?;
    assert!(read(&profile)?.projects.is_empty());
    std::fs::remove_dir_all(profile)
}
