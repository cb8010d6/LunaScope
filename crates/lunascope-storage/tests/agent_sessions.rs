use lunascope_core::{
    AgentSessionId, AgentSessionKind, AgentSessionRecord, AgentSessionState, ProjectId, RunId,
    RunLeaseRecord, ThreadId, WorkerId,
};
use lunascope_storage::SqliteEventStore;

fn session(id: &str, parent: Option<&str>, state: AgentSessionState) -> AgentSessionRecord {
    AgentSessionRecord {
        session_id: AgentSessionId::from(id),
        parent_session_id: parent.map(AgentSessionId::from),
        project_id: ProjectId::from("project-agent-sessions"),
        thread_id: ThreadId::from("thread-agent-sessions"),
        run_id: RunId::from("run-agent-sessions"),
        worker_id: (id == "worker-session").then(|| WorkerId::from("worker-1")),
        kind: if parent.is_some() {
            AgentSessionKind::Worker
        } else {
            AgentSessionKind::Primary
        },
        display_name: if parent.is_some() {
            "Implement parser".into()
        } else {
            "LunaScope".into()
        },
        state,
        created_at: "2026-08-09T00:00:00Z".into(),
        updated_at: "2026-08-09T00:00:01Z".into(),
    }
}

#[test]
fn run_leases_are_renewed_and_expired_deterministically() {
    let store = SqliteEventStore::open_in_memory().expect("store");
    let run_id = RunId::new("run-lease");
    let initial = RunLeaseRecord {
        run_id: run_id.clone(),
        owner_id: "desktop-1".into(),
        phase: "planning".into(),
        heartbeat_at: "2026-08-09T10:00:00Z".into(),
        expires_at: "2026-08-09T10:00:30Z".into(),
        last_progress_at: "2026-08-09T10:00:00Z".into(),
    };
    store.save_run_lease(&initial).expect("save lease");
    assert!(
        store
            .expired_run_leases("2026-08-09T10:00:20Z")
            .expect("query active leases")
            .is_empty()
    );

    let renewed = RunLeaseRecord {
        phase: "running".into(),
        heartbeat_at: "2026-08-09T10:00:20Z".into(),
        expires_at: "2026-08-09T10:00:50Z".into(),
        last_progress_at: "2026-08-09T10:00:18Z".into(),
        ..initial
    };
    store.save_run_lease(&renewed).expect("renew lease");
    assert_eq!(store.run_lease(&run_id).expect("load lease"), Some(renewed));
    assert_eq!(
        store
            .expired_run_leases("2026-08-09T10:00:51Z")
            .expect("query expired leases")
            .len(),
        1
    );
    assert!(store.release_run_lease(&run_id).expect("release lease"));
    assert!(
        store
            .run_lease(&run_id)
            .expect("load released lease")
            .is_none()
    );
}

#[test]
fn agent_session_tree_is_durable_and_updatable() {
    let store = SqliteEventStore::open_in_memory().expect("store");
    store
        .save_agent_session(&session(
            "primary-session",
            None,
            AgentSessionState::Running,
        ))
        .expect("save primary");
    store
        .save_agent_session(&session(
            "worker-session",
            Some("primary-session"),
            AgentSessionState::WaitingForModel,
        ))
        .expect("save worker");

    let mut worker = session(
        "worker-session",
        Some("primary-session"),
        AgentSessionState::Completed,
    );
    worker.updated_at = "2026-08-09T00:01:00Z".into();
    store.save_agent_session(&worker).expect("update worker");

    let sessions = store
        .agent_sessions_for_run(&RunId::from("run-agent-sessions"))
        .expect("load sessions");
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[1].parent_session_id,
        Some(AgentSessionId::from("primary-session"))
    );
    assert_eq!(sessions[1].state, AgentSessionState::Completed);
}
