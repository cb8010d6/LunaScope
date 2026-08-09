use std::path::Path;

use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, McpConfigValue, McpHeader,
    McpServerConfig, McpTransportConfig, ModelAssignment, ModelCapability, ModelRole,
    ModelRoutingPolicy, ModelSelectionSettings, ProjectId, ProviderConfig, ProviderProtocol,
    ProviderType, ReasoningEffort, RoutingPriorities, RunId, RunState, ThreadId,
};
use lunascope_storage::{AppendOutcome, SqliteEventStore, StorageError};

fn event(sequence: u64, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(format!("evt-{sequence}")),
        sequence,
        format!("2026-07-27T00:00:{sequence:02}Z"),
        ProjectId::from("project-1"),
        ThreadId::from("thread-1"),
        RunId::from("run-1"),
        CorrelationId::from("correlation-1"),
        EventSource::System,
        payload,
    )
}

fn generated_event(suffix: &str, payload: EventData) -> EventEnvelope {
    let mut value = event(1, payload);
    value.sequence = 0;
    value.event_id = EventId::new(format!("generated-{suffix}"));
    value
}

fn append_run_start(store: &SqliteEventStore) {
    store
        .append(&event(
            1,
            EventData::RunCreated {
                title: "Recovery test".into(),
                initial_prompt: "Persist me".into(),
            },
        ))
        .expect("append run created");
    store
        .append(&event(
            2,
            EventData::RunStateChanged {
                from: RunState::Created,
                to: RunState::Planning,
                reason: "begin planning".into(),
            },
        ))
        .expect("append planning");
}

#[test]
fn appends_in_order_and_updates_projection_transactionally() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    append_run_start(&store);
    let projection = store
        .projection(&RunId::from("run-1"))
        .expect("load projection")
        .expect("projection exists");
    assert_eq!(projection.sequence, 2);
    assert_eq!(projection.run_state, RunState::Planning);
}

#[test]
fn rejects_sequence_gaps_without_mutating_projection() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    append_run_start(&store);
    let error = store
        .append(&event(
            4,
            EventData::RunStateChanged {
                from: RunState::Planning,
                to: RunState::Running,
                reason: "skip sequence 3".into(),
            },
        ))
        .expect_err("gap must fail");
    assert!(matches!(
        error,
        StorageError::Projection(lunascope_core::ProjectionError::SequenceGap {
            expected: 3,
            actual: 4
        })
    ));
    assert_eq!(
        store
            .projection(&RunId::from("run-1"))
            .expect("load projection")
            .expect("projection exists")
            .sequence,
        2
    );
}

#[test]
fn exact_duplicate_event_is_idempotent_but_conflict_is_rejected() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    let original = event(
        1,
        EventData::RunCreated {
            title: "Original".into(),
            initial_prompt: "Persist me".into(),
        },
    );
    assert_eq!(
        store.append(&original).expect("first append"),
        AppendOutcome::Appended { sequence: 1 }
    );
    assert_eq!(
        store.append(&original).expect("duplicate append"),
        AppendOutcome::Duplicate { sequence: 1 }
    );

    let conflicting = event(
        1,
        EventData::RunCreated {
            title: "Changed".into(),
            initial_prompt: "same id, different content".into(),
        },
    );
    assert!(matches!(
        store.append(&conflicting),
        Err(StorageError::ConflictingEventId(_))
    ));
}

#[test]
fn generated_batch_allocates_sequences_and_rolls_back_as_one_unit() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    let committed = store
        .append_batch_next(vec![
            generated_event(
                "created",
                EventData::RunCreated {
                    title: "Atomic batch".into(),
                    initial_prompt: "test".into(),
                },
            ),
            generated_event(
                "planning",
                EventData::RunStateChanged {
                    from: RunState::Created,
                    to: RunState::Planning,
                    reason: "plan".into(),
                },
            ),
        ])
        .expect("append generated batch");
    assert_eq!(
        committed
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let result = store.append_batch_next(vec![
        generated_event(
            "running",
            EventData::RunStateChanged {
                from: RunState::Planning,
                to: RunState::Running,
                reason: "run".into(),
            },
        ),
        generated_event(
            "invalid",
            EventData::RunStateChanged {
                from: RunState::Running,
                to: RunState::Completed,
                reason: "invalid direct completion".into(),
            },
        ),
    ]);
    assert!(result.is_err());
    let projection = store
        .projection(&RunId::from("run-1"))
        .expect("projection")
        .expect("run exists");
    assert_eq!(projection.sequence, 2);
    assert_eq!(projection.run_state, RunState::Planning);
}

#[test]
fn restart_recovers_from_snapshot_plus_committed_events() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("events.db");
    write_recovery_fixture(&path);

    let reopened = SqliteEventStore::open(&path).expect("reopen store");
    let recovered = reopened
        .recover(&RunId::from("run-1"))
        .expect("recover")
        .expect("run exists");
    assert_eq!(recovered.sequence, 5);
    assert_eq!(recovered.run_state, RunState::Paused);
}

fn write_recovery_fixture(path: &Path) {
    let store = SqliteEventStore::open(path).expect("open file store");
    append_run_start(&store);
    store
        .create_snapshot(&RunId::from("run-1"))
        .expect("create snapshot");
    store
        .append(&event(
            3,
            EventData::RunStateChanged {
                from: RunState::Planning,
                to: RunState::Running,
                reason: "execute".into(),
            },
        ))
        .expect("append running");
    store
        .append(&event(
            4,
            EventData::RunStateChanged {
                from: RunState::Running,
                to: RunState::Pausing,
                reason: "pause requested".into(),
            },
        ))
        .expect("append pausing");
    store
        .append(&event(
            5,
            EventData::RunStateChanged {
                from: RunState::Pausing,
                to: RunState::Paused,
                reason: "workers quiesced".into(),
            },
        ))
        .expect("append paused");
}

#[test]
fn provider_metadata_and_routing_policy_survive_restart() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("settings.db");
    let config = ProviderConfig {
        id: "openai-primary".into(),
        provider_type: ProviderType::OpenAi,
        protocol: ProviderProtocol::OpenAiResponses,
        display_name: "OpenAI".into(),
        base_url: "https://api.openai.com".into(),
        credential_reference_id: "credential-openai-primary".into(),
        default_model_id: "gpt-5-mini".into(),
        custom_headers: Vec::new(),
        context_window_tokens: Some(128_000),
        supports_tools: true,
        supports_vision: true,
        supports_structured_output: true,
        enabled: true,
    };
    let policy = ModelRoutingPolicy {
        priorities: RoutingPriorities {
            quality: 90,
            cost: 50,
            speed: 40,
            privacy: 60,
        },
        allowed_provider_config_ids: vec![config.id.clone()],
        disallowed_provider_config_ids: Vec::new(),
        maximum_cost_microusd_per_run: Some(50_000),
        prefer_local: false,
        required_capabilities: vec![ModelCapability::Text],
        fallback_allowed: true,
        ask_before_cost_escalation: true,
        role_preferences: Vec::new(),
    };
    let settings = ModelSelectionSettings {
        orchestration: ModelAssignment {
            role: ModelRole::Orchestration,
            provider_config_id: config.id.clone(),
            model_id: config.default_model_id.clone(),
            custom_reasoning_effort: None,
            reasoning_effort: ReasoningEffort::High,
            maximum_context_tokens: Some(128_000),
            maximum_budget_microusd: Some(50_000),
            fallback_provider_config_id: None,
            fallback_model_id: None,
            locked: true,
        },
        vision: None,
        worker_pool: vec![ModelAssignment {
            role: ModelRole::Programming,
            provider_config_id: config.id.clone(),
            model_id: config.default_model_id.clone(),
            custom_reasoning_effort: None,
            reasoning_effort: ReasoningEffort::Medium,
            maximum_context_tokens: Some(64_000),
            maximum_budget_microusd: Some(20_000),
            fallback_provider_config_id: None,
            fallback_model_id: None,
            locked: false,
        }],
        custom_reasoning_efforts: Default::default(),
    };
    {
        let store = SqliteEventStore::open(&path).expect("store");
        store.save_provider_config(&config).expect("save config");
        store
            .save_routing_policy("global", &policy)
            .expect("save policy");
        store
            .save_model_selection_settings("global", &settings)
            .expect("save model settings");
    }
    let reopened = SqliteEventStore::open(&path).expect("reopen");
    assert_eq!(reopened.provider_configs().unwrap(), vec![config]);
    assert_eq!(
        reopened.routing_policy("global").unwrap().as_ref(),
        Some(&policy)
    );
    assert_eq!(
        reopened
            .model_selection_settings("global")
            .unwrap()
            .as_ref(),
        Some(&settings)
    );
}

#[test]
fn mcp_server_configs_survive_restart_and_can_be_deleted() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("mcp-settings.db");
    let config = McpServerConfig {
        id: "docs-remote".into(),
        name: "Documentation MCP".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: "https://example.com/mcp".into(),
            headers: vec![McpHeader {
                name: "Authorization".into(),
                value: McpConfigValue::CredentialReference("bearer".into()),
            }],
        },
        timeout_ms: 15_000,
        enabled: true,
    };
    {
        let store = SqliteEventStore::open(&path).expect("store");
        store
            .save_mcp_server_config(&config)
            .expect("save MCP config");
    }
    let reopened = SqliteEventStore::open(&path).expect("reopen");
    assert_eq!(reopened.mcp_server_configs().unwrap(), vec![config]);
    assert!(
        reopened
            .delete_mcp_server_config("docs-remote")
            .expect("delete MCP config")
    );
    assert!(
        !reopened
            .delete_mcp_server_config("docs-remote")
            .expect("delete missing MCP config")
    );
    assert!(reopened.mcp_server_configs().unwrap().is_empty());
}
