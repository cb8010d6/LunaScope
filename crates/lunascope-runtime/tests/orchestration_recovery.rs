use lunascope_core::{
    AllowedWorkerModel, CorrelationId, EventData, EventEnvelope, EventId, EventSource,
    OrchestrationPatch, OrchestrationPatchApplyMode, OrchestrationPatchOperation, ProjectId, RunId,
    RunState, ThreadId, WorkerField, WorkerPatch,
};
use lunascope_runtime::{apply_user_patch, draft_orchestration};
use lunascope_storage::SqliteEventStore;
use uuid::Uuid;

fn event(run_id: &RunId, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(Uuid::new_v4().to_string()),
        0,
        jiff::Timestamp::now().to_string(),
        ProjectId::from("orchestration-recovery-test"),
        ThreadId::from("thread-orchestration"),
        run_id.clone(),
        CorrelationId::from("orchestration-correlation"),
        EventSource::Orchestrator,
        payload,
    )
}

#[test]
fn worker_graph_and_user_locked_patch_survive_database_reopen() {
    let temporary = tempfile::tempdir().expect("temporary database");
    let database = temporary.path().join("runtime.db");
    let run_id = RunId::from("run-orchestration-recovery");
    let plan = draft_orchestration(
        "Research and implement multiple modules, then verify",
        vec![AllowedWorkerModel {
            provider: "deepseek-primary".into(),
            model: "deepseek-v4-flash".into(),
        }],
        vec![
            "filesystem_read".into(),
            "filesystem_write".into(),
            "process_spawn".into(),
        ],
    )
    .expect("draft");
    let worker_id = plan.workers[0].worker_id.clone();
    let store = SqliteEventStore::open(&database).expect("store");
    let mut initial = vec![
        event(
            &run_id,
            EventData::RunCreated {
                title: "Durable orchestration".into(),
                initial_prompt: plan.objective.clone(),
            },
        ),
        event(
            &run_id,
            EventData::RunStateChanged {
                from: RunState::Created,
                to: RunState::Planning,
                reason: "draft graph".into(),
            },
        ),
        event(
            &run_id,
            EventData::OrchestrationCreated {
                plan: Box::new(plan.clone()),
            },
        ),
    ];
    initial.extend(plan.workers.iter().cloned().map(|spec| {
        event(
            &run_id,
            EventData::WorkerCreated {
                spec: Box::new(spec),
            },
        )
    }));
    store.append_batch_next(initial).expect("initial graph");

    let patch = OrchestrationPatch {
        patch_id: "patch-user-locked".into(),
        base_version: plan.version,
        apply_mode: OrchestrationPatchApplyMode::ApplyNow,
        reason: "User owns this prompt".into(),
        operations: vec![OrchestrationPatchOperation::UpdateWorker {
            patch: WorkerPatch {
                worker_id: worker_id.clone(),
                display_name: None,
                role: None,
                tags: Some(vec!["user-owned".into()]),
                objective: None,
                task: None,
                prompt: Some("USER DURABLE PROMPT".into()),
                input_context: None,
                expected_output: None,
                output_schema: None,
                completion_criteria: None,
                owned_acceptance_criteria: None,
                parallel_group: None,
                model: None,
                skills: None,
                tools: None,
                permissions: None,
                budget: None,
                dependencies: None,
                timeout_ms: None,
                retry_policy: None,
                checkpoint_policy: None,
                write_scopes: None,
                parent_worker_id: None,
                lock_fields: vec![WorkerField::Prompt, WorkerField::Tags],
                unlock_fields: Vec::new(),
            },
        }],
    };
    let patched = apply_user_patch(&plan, &patch).expect("patch");
    store
        .append_batch_next(vec![event(
            &run_id,
            EventData::OrchestrationPatched {
                patch: Box::new(patch),
                plan: Box::new(patched.clone()),
            },
        )])
        .expect("persist patch");
    drop(store);

    let reopened = SqliteEventStore::open(&database).expect("reopen");
    let snapshot = reopened
        .recover(&run_id)
        .expect("recover")
        .expect("snapshot");
    let recovered = snapshot.orchestration_plan.expect("durable plan");
    assert_eq!(recovered.version, 2);
    let worker = recovered
        .workers
        .iter()
        .find(|worker| worker.worker_id == worker_id)
        .expect("worker");
    assert_eq!(worker.prompt, "USER DURABLE PROMPT");
    assert!(worker.locked_fields.contains(&WorkerField::Prompt));
    assert_eq!(worker.tags, vec!["user-owned"]);
    assert!(worker.locked_fields.contains(&WorkerField::Tags));
    assert_eq!(recovered.user_overrides.len(), 1);
    assert_eq!(snapshot.workers.len(), plan.workers.len());
}
