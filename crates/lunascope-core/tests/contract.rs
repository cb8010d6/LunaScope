use std::{fs, path::PathBuf};

use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, EventType, ProjectId, RunId,
    RunState, ThreadId, WorkerState,
};

fn workspace_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate must live under workspace/crates")
        .join(relative)
}

fn sample_event() -> EventEnvelope {
    EventEnvelope::new(
        EventId::from("evt-1"),
        1,
        "2026-07-27T00:00:00Z",
        ProjectId::from("project-1"),
        ThreadId::from("thread-1"),
        RunId::from("run-1"),
        CorrelationId::from("correlation-1"),
        EventSource::System,
        EventData::RunCreated {
            title: "Contract test".into(),
            initial_prompt: "Verify the persisted envelope".into(),
        },
    )
}

#[test]
fn envelope_constructor_keeps_type_and_payload_consistent() {
    let event = sample_event();
    assert_eq!(event.event_type, EventType::RunCreated);
    assert_eq!(event.schema_version, 1);
    assert!(event.validate().is_ok());
}

#[test]
fn envelope_rejects_type_mismatch_and_zero_sequence() {
    let mut event = sample_event();
    event.event_type = EventType::RunFailed;
    assert!(event.validate().is_err());

    event.event_type = EventType::RunCreated;
    event.sequence = 0;
    assert!(event.validate().is_err());
}

#[test]
fn state_change_event_rejects_an_impossible_transition() {
    let mut event = sample_event();
    event.event_type = EventType::RunStateChanged;
    event.payload = EventData::RunStateChanged {
        from: RunState::Completed,
        to: RunState::Running,
        reason: "terminal runs cannot restart".into(),
    };
    assert!(event.validate().is_err());
}

#[test]
fn serialization_uses_public_contract_casing() {
    let value = serde_json::to_value(sample_event()).expect("event serializes");
    assert_eq!(value["eventType"], "run_created");
    assert_eq!(value["payload"]["kind"], "run_created");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["redactionState"], "not_required");
}

#[test]
fn provider_config_added_metadata_is_backward_compatible() {
    let config: lunascope_core::ProviderConfig = serde_json::from_value(serde_json::json!({
        "id": "legacy-openai",
        "providerType": "open_ai",
        "protocol": "open_ai_responses",
        "displayName": "Legacy OpenAI",
        "baseUrl": "https://api.openai.com",
        "credentialReferenceId": "legacy-openai-key",
        "customHeaders": [],
        "enabled": true
    }))
    .expect("legacy provider config deserializes");

    assert_eq!(config.default_model_id, "");
    assert_eq!(config.context_window_tokens, None);
    assert!(!config.supports_tools);
    assert!(!config.supports_vision);
    assert!(!config.supports_structured_output);
}

#[test]
fn run_state_machine_accepts_recovery_path_and_rejects_terminal_restart() {
    assert!(
        RunState::Created
            .validate_transition(RunState::Planning)
            .is_ok()
    );
    assert!(
        RunState::Planning
            .validate_transition(RunState::Running)
            .is_ok()
    );
    assert!(
        RunState::Running
            .validate_transition(RunState::Pausing)
            .is_ok()
    );
    assert!(
        RunState::Pausing
            .validate_transition(RunState::Paused)
            .is_ok()
    );
    assert!(
        RunState::Paused
            .validate_transition(RunState::Running)
            .is_ok()
    );
    assert!(
        RunState::Completed
            .validate_transition(RunState::Running)
            .is_err()
    );
}

#[test]
fn worker_state_machine_requires_approval_before_tool_execution() {
    assert!(
        WorkerState::RunningModel
            .validate_transition(WorkerState::WaitingToolApproval)
            .is_ok()
    );
    assert!(
        WorkerState::WaitingToolApproval
            .validate_transition(WorkerState::RunningTool)
            .is_ok()
    );
    assert!(
        WorkerState::Draft
            .validate_transition(WorkerState::RunningTool)
            .is_err()
    );
}

#[test]
fn checked_in_typescript_contract_is_current() {
    let path = workspace_path("packages/runtime-contract/src/types.generated.ts");
    let checked_in = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    assert_eq!(
        checked_in,
        lunascope_core::typescript_contract(),
        "run `cargo run -p lunascope-core --example export_contract`"
    );
}

#[test]
fn checked_in_json_schema_is_current_and_requires_durable_fields() {
    let path = workspace_path("packages/runtime-contract/runtime-event.schema.json");
    let checked_in = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    assert_eq!(
        checked_in,
        lunascope_core::event_schema().expect("schema serializes"),
        "run `cargo run -p lunascope-core --example export_contract`"
    );

    let schema: serde_json::Value =
        serde_json::from_str(&checked_in).expect("checked-in schema is JSON");
    let required = schema["required"]
        .as_array()
        .expect("event envelope has required fields");
    for field in [
        "eventId",
        "sequence",
        "schemaVersion",
        "timestamp",
        "projectId",
        "threadId",
        "runId",
        "correlationId",
        "eventType",
        "source",
        "payload",
        "risk",
        "redactionState",
    ] {
        assert!(
            required.iter().any(|value| value == field),
            "schema must require {field}"
        );
    }
}
