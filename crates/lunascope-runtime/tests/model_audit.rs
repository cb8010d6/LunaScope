use std::sync::Arc;

use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, ModelRole, ModelRoutingDecision,
    ModelUsageRecord, ProjectId, RunId, ThreadId,
};
use lunascope_runtime::{DurableApprovalRuntime, RunIdentity};
use lunascope_storage::SqliteEventStore;

fn identity() -> RunIdentity {
    RunIdentity {
        project_id: ProjectId::from("project-model-audit"),
        thread_id: ThreadId::from("thread-model-audit"),
        run_id: RunId::from("run-model-audit"),
        correlation_id: CorrelationId::from("correlation-model-audit"),
    }
}

fn created_event() -> EventEnvelope {
    EventEnvelope::new(
        EventId::new("model-audit-created"),
        0,
        "2026-07-27T10:00:00Z",
        ProjectId::from("project-model-audit"),
        ThreadId::from("thread-model-audit"),
        RunId::from("run-model-audit"),
        CorrelationId::from("correlation-model-audit"),
        EventSource::System,
        EventData::RunCreated {
            title: "Model audit".into(),
            initial_prompt: "Persist model routing and usage".into(),
        },
    )
}

#[test]
fn routing_decision_and_usage_survive_database_reopen() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("model-audit.db");
    let decision = ModelRoutingDecision {
        provider_config_id: "anthropic-primary".into(),
        model_id: "claude-test".into(),
        reason: "writing role prefers Anthropic".into(),
        fallback: false,
        estimated_cost_microusd: 84,
        requires_cost_approval: false,
    };
    let usage = ModelUsageRecord {
        provider_config_id: "anthropic-primary".into(),
        model_id: "claude-test".into(),
        role: ModelRole::Writing,
        input_tokens: 41,
        output_tokens: 17,
        total_cost_microusd: Some(84),
        fallback_from: None,
    };

    {
        let store = Arc::new(SqliteEventStore::open(&path).expect("event store"));
        store
            .append_batch_next(vec![created_event()])
            .expect("initialize durable run");
        let runtime = DurableApprovalRuntime::new(store, identity());
        runtime
            .record_model_routing(decision.clone())
            .expect("record routing");
        runtime
            .record_model_usage(usage.clone())
            .expect("record usage");
    }

    let reopened = SqliteEventStore::open(&path).expect("reopen event store");
    let events = reopened
        .events_after(&RunId::from("run-model-audit"), 0)
        .expect("read audit events");
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventData::ModelRoutingDecided { decision: recorded } if recorded == &decision
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventData::ModelUsageRecorded { usage: recorded } if recorded == &usage
        )
    }));
}
