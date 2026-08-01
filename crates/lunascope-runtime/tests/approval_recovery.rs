use std::sync::Arc;

use lunascope_core::{
    ApprovalRequest, ApprovalResolution, CorrelationId, EventData, EventEnvelope, EventId,
    EventSource, PermissionDecision, ProjectId, RiskLevel, RunId, RunState, ThreadId, ToolCall,
};
use lunascope_runtime::{DurableApprovalRuntime, RunIdentity};
use lunascope_storage::SqliteEventStore;

fn identity() -> RunIdentity {
    RunIdentity {
        project_id: ProjectId::from("project-approval"),
        thread_id: ThreadId::from("thread-approval"),
        run_id: RunId::from("run-approval"),
        correlation_id: CorrelationId::from("correlation-approval"),
    }
}

fn event(id: &str, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(id),
        0,
        "2026-07-27T08:00:00Z",
        ProjectId::from("project-approval"),
        ThreadId::from("thread-approval"),
        RunId::from("run-approval"),
        CorrelationId::from("correlation-approval"),
        EventSource::System,
        payload,
    )
}

fn initialize_running(store: &SqliteEventStore) {
    store
        .append_batch_next(vec![
            event(
                "created",
                EventData::RunCreated {
                    title: "Approval recovery".into(),
                    initial_prompt: "edit a file".into(),
                },
            ),
            event(
                "planning",
                EventData::RunStateChanged {
                    from: RunState::Created,
                    to: RunState::Planning,
                    reason: "plan".into(),
                },
            ),
            event(
                "running",
                EventData::RunStateChanged {
                    from: RunState::Planning,
                    to: RunState::Running,
                    reason: "execute".into(),
                },
            ),
        ])
        .expect("initialize run");
}

#[test]
fn approval_interrupt_survives_restart_and_rejection_is_recorded() {
    let directory = tempfile::tempdir().expect("temp directory");
    let database = directory.path().join("events.db");
    {
        let store = Arc::new(SqliteEventStore::open(&database).expect("open store"));
        initialize_running(&store);
        let runtime = DurableApprovalRuntime::new(store, identity());
        let waiting = runtime
            .request(
                ToolCall {
                    call_id: "call-write-1".into(),
                    tool_id: "filesystem.apply_text_patch".into(),
                    input: serde_json::json!({"path": "src/lib.rs"}),
                    idempotency_key: "write-1".into(),
                    timeout_ms: 30_000,
                },
                ApprovalRequest {
                    approval_id: "approval-write-1".into(),
                    call_id: None,
                    action: "Patch src/lib.rs".into(),
                    reason: "write changes project state".into(),
                    permissions: vec!["filesystem_write".into()],
                    scope: "workspace".into(),
                    blast_radius: "one file".into(),
                    rollback: "restore exact prior bytes".into(),
                },
                RiskLevel::Medium,
            )
            .expect("request approval");
        assert_eq!(waiting.run_state, RunState::WaitingApproval);
        assert_eq!(waiting.pending_approvals.len(), 1);
        assert_eq!(waiting.sequence, 6);
    }

    let reopened = Arc::new(SqliteEventStore::open(&database).expect("reopen store"));
    let runtime = DurableApprovalRuntime::new(reopened.clone(), identity());
    let recovered = runtime.recover().expect("recover");
    assert_eq!(recovered.run_state, RunState::WaitingApproval);
    assert_eq!(
        runtime
            .pending_tool_call("approval-write-1")
            .expect("recover tool call")
            .call_id,
        "call-write-1"
    );

    let resumed = runtime
        .resolve(ApprovalResolution {
            approval_id: "approval-write-1".into(),
            decision: PermissionDecision::Reject,
            instructions: Some("do not modify this file".into()),
        })
        .expect("reject approval");
    assert!(resumed.is_none());
    let snapshot = runtime
        .checkpoint("approval rejection handled")
        .expect("checkpoint");
    assert_eq!(snapshot.run_state, RunState::Running);
    assert!(snapshot.pending_approvals.is_empty());
    assert_eq!(snapshot.sequence, 10);

    drop(runtime);
    drop(reopened);
    let final_store = SqliteEventStore::open(&database).expect("final reopen");
    let final_snapshot = final_store
        .recover(&RunId::from("run-approval"))
        .expect("final recovery")
        .expect("run exists");
    assert_eq!(final_snapshot, snapshot);
    let events = final_store
        .events_after(&RunId::from("run-approval"), 0)
        .expect("events");
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventData::ToolCallCompleted { result, .. }
                if !result.success
                    && result.error_code.as_deref() == Some("approval_rejected")
        )
    }));
}
