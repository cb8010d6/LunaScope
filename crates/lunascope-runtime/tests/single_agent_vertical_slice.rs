use std::{collections::BTreeMap, fs, sync::Arc, time::Duration};

use lunascope_core::{
    ApprovalRequest, ApprovalResolution, CorrelationId, EventData, EventEnvelope, EventId,
    EventSource, PermissionContext, PermissionDecision, PermissionKind, PermissionRequest,
    PolicyDecision, ProjectId, RiskLevel, RunId, RunState, ThreadId, ToolCall, ToolResult,
    VerificationRecord, VerificationStatus,
};
use lunascope_runtime::{
    DurableApprovalRuntime, FilesystemTools, OpenAiResponseRequest, OpenAiResponsesProvider,
    PolicyEngine, ProcessExecutor, ProcessSpec, ProviderStreamEvent, RunIdentity, TextPatch,
};
use lunascope_storage::SqliteEventStore;
use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

fn identity() -> RunIdentity {
    RunIdentity {
        project_id: ProjectId::from("project-vertical-slice"),
        thread_id: ThreadId::from("thread-vertical-slice"),
        run_id: RunId::from("run-vertical-slice"),
        correlation_id: CorrelationId::from("correlation-vertical-slice"),
    }
}

fn event(id: &str, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(id),
        0,
        "2026-07-27T09:00:00Z",
        ProjectId::from("project-vertical-slice"),
        ThreadId::from("thread-vertical-slice"),
        RunId::from("run-vertical-slice"),
        CorrelationId::from("correlation-vertical-slice"),
        EventSource::System,
        payload,
    )
}

async fn execute(
    executor: &ProcessExecutor,
    root: &str,
    program: &str,
    args: &[&str],
) -> lunascope_runtime::ProcessOutput {
    executor
        .execute(
            &ProcessSpec {
                program: program.into(),
                args: args.iter().map(|value| (*value).into()).collect(),
                cwd: root.into(),
                env: BTreeMap::new(),
                timeout_ms: 30_000,
            },
            CancellationToken::new(),
        )
        .await
        .expect("execute process")
}

#[tokio::test]
async fn approved_patch_runs_tests_and_persists_verification_evidence() {
    let directory = tempfile::tempdir().expect("temp directory");
    let root = directory.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("create repository");
    fs::write(
        root.join("src/math.ps1"),
        "function Add-One {\n    param([int]$Value)\n    return $Value\n}\n",
    )
    .expect("write source");
    fs::write(
        root.join("test.ps1"),
        ". \"$PSScriptRoot/src/math.ps1\"\n\
         if ((Add-One 41) -ne 42) { throw 'expected 42' }\n\
         Write-Output 'PASS Add-One returns 42'\n",
    )
    .expect("write test");

    let root_text = root.display().to_string();
    let executor =
        ProcessExecutor::new(&root, ["git.exe", "powershell.exe"]).expect("process executor");
    for args in [
        vec!["init"],
        vec!["config", "user.name", "LunaScope Test"],
        vec!["config", "user.email", "lunascope@example.invalid"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture"],
    ] {
        let output = execute(&executor, &root_text, "git.exe", &args).await;
        assert_eq!(output.exit_code, Some(0), "git failed: {}", output.stderr);
    }

    let store =
        Arc::new(SqliteEventStore::open(directory.path().join("events.db")).expect("event store"));
    store
        .append_batch_next(vec![
            event(
                "created",
                EventData::RunCreated {
                    title: "Repair Add-One".into(),
                    initial_prompt: "Make the failing test pass".into(),
                },
            ),
            event(
                "planning",
                EventData::RunStateChanged {
                    from: RunState::Created,
                    to: RunState::Planning,
                    reason: "inspect defect".into(),
                },
            ),
            event(
                "running",
                EventData::RunStateChanged {
                    from: RunState::Planning,
                    to: RunState::Running,
                    reason: "execute approved plan".into(),
                },
            ),
        ])
        .expect("initialize event stream");
    let runtime = DurableApprovalRuntime::new(store.clone(), identity());
    let filesystem = FilesystemTools::new(&root).expect("filesystem tools");
    let original = filesystem
        .read_range("src/math.ps1", 0, 4096)
        .expect("read source");
    let provider_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind provider fixture");
    let provider_address = provider_listener.local_addr().expect("provider address");
    let provider_arguments = serde_json::json!({
        "path": "src/math.ps1",
        "expected_sha256": original.sha256,
        "replacements": [{
            "old": "return $Value",
            "new": "return $Value + 1",
            "expected_occurrences": 1
        }]
    });
    let provider_server = tokio::spawn(serve_provider_call(
        provider_listener,
        provider_arguments.clone(),
    ));
    let provider = OpenAiResponsesProvider::new(
        Url::parse(&format!("http://{provider_address}/v1/responses")).expect("provider URL"),
        "vertical-slice-test-credential",
    )
    .expect("native provider");
    let provider_summary = provider
        .stream(
            &OpenAiResponseRequest {
                model: "test-model".into(),
                input: serde_json::json!({
                    "task": "Make the failing Add-One test pass",
                    "source": original.text,
                }),
                instructions: Some(
                    "Use filesystem.apply_text_patch with a guarded exact patch.".into(),
                ),
                tools: vec![serde_json::json!({
                    "type": "function",
                    "name": "filesystem.apply_text_patch",
                    "strict": true,
                })],
                max_output_tokens: Some(512),
            },
            Duration::from_secs(5),
            CancellationToken::new(),
            |provider_event| {
                if let ProviderStreamEvent::TextDelta { sequence, delta } = provider_event {
                    runtime
                        .record_model_chunk("resp_vertical_slice", sequence, delta)
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            },
        )
        .await
        .expect("stream native provider");
    provider_server.await.expect("provider server task");
    assert_eq!(provider_summary.text, "I found the off-by-one defect.");
    let function_call = provider_summary
        .function_calls
        .first()
        .expect("provider returned a function call");
    assert_eq!(function_call.name, "filesystem.apply_text_patch");
    let requested_patch: TextPatch =
        serde_json::from_value(function_call.arguments.clone()).expect("valid tool arguments");

    let policy_request = PermissionRequest {
        permission: PermissionKind::FilesystemWrite,
        context: PermissionContext {
            workspace_root: root_text.clone(),
            target_path: Some(root.join("src/math.ps1").display().to_string()),
            tool_id: Some("filesystem.apply_text_patch".into()),
            ..PermissionContext::default()
        },
        risk: RiskLevel::Medium,
        action: "Correct Add-One return value".into(),
    };
    assert_eq!(
        PolicyEngine::default().evaluate(&policy_request).decision,
        PolicyDecision::Ask
    );

    let call = ToolCall {
        call_id: "call-patch-add-one".into(),
        tool_id: function_call.name.clone(),
        input: function_call.arguments.clone(),
        idempotency_key: "patch-add-one-v1".into(),
        timeout_ms: 30_000,
    };
    runtime
        .request(
            call.clone(),
            ApprovalRequest {
                approval_id: "approval-patch-add-one".into(),
                call_id: None,
                action: "Patch src/math.ps1".into(),
                reason: "source write requires approval".into(),
                permissions: vec!["filesystem_write".into()],
                scope: root_text.clone(),
                blast_radius: "one tracked source file".into(),
                rollback: "git restore src/math.ps1".into(),
            },
            RiskLevel::Medium,
        )
        .expect("request approval");
    assert_eq!(
        runtime
            .resolve(ApprovalResolution {
                approval_id: "approval-patch-add-one".into(),
                decision: PermissionDecision::AllowOnce,
                instructions: None,
            })
            .expect("approve call"),
        Some(call)
    );

    let patch = filesystem
        .apply_text_patch(&requested_patch)
        .expect("apply guarded patch");
    let test = execute(
        &executor,
        &root_text,
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "test.ps1",
        ],
    )
    .await;
    let diff_check = execute(&executor, &root_text, "git.exe", &["diff", "--check"]).await;
    let diff = execute(
        &executor,
        &root_text,
        "git.exe",
        &["diff", "--", "src/math.ps1"],
    )
    .await;
    let verified = test.exit_code == Some(0)
        && diff_check.exit_code == Some(0)
        && diff.stdout.contains("return $Value + 1");
    assert!(verified, "test={} diff={}", test.stderr, diff.stdout);

    runtime
        .record_tool_result(
            ToolResult {
                call_id: "call-patch-add-one".into(),
                success: true,
                output: serde_json::json!({
                    "path": patch.path,
                    "sha256": patch.sha256,
                    "test": test.stdout.trim(),
                    "diffCheckExitCode": diff_check.exit_code,
                }),
                error_code: None,
                duration_ms: test.duration_ms + diff_check.duration_ms,
            },
            RiskLevel::Medium,
        )
        .expect("record tool result");
    let verified_snapshot = runtime
        .record_verification(VerificationRecord {
            status: VerificationStatus::Verified,
            summary: "PowerShell regression test and git diff check passed".into(),
            evidence: vec![test.stdout.trim().into(), "git diff --check: exit 0".into()],
            remaining_risks: Vec::new(),
            criterion_results: Vec::new(),
            findings: Vec::new(),
        })
        .expect("record verification");
    assert_eq!(verified_snapshot.verification, VerificationStatus::Verified);
    let checkpoint = runtime
        .checkpoint("single-agent vertical slice verified")
        .expect("checkpoint");
    assert_eq!(checkpoint.run_state, RunState::Running);

    drop(runtime);
    drop(store);
    let reopened = SqliteEventStore::open(directory.path().join("events.db")).expect("reopen");
    let recovered = reopened
        .recover(&RunId::from("run-vertical-slice"))
        .expect("recover")
        .expect("run exists");
    assert_eq!(recovered, checkpoint);
    assert_eq!(recovered.verification, VerificationStatus::Verified);
}

async fn serve_provider_call(listener: TcpListener, arguments: serde_json::Value) {
    let (mut socket, _) = listener.accept().await.expect("accept provider call");
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = socket
            .read(&mut buffer)
            .await
            .expect("read provider request");
        assert!(read > 0, "provider request ended before headers");
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let arguments = serde_json::to_string(&arguments).expect("serialize function arguments");
    let events = [
        serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_vertical",
            "output_index": 0,
            "content_index": 0,
            "delta": "I found the off-by-one defect."
        }),
        serde_json::json!({
            "type": "response.function_call_arguments.done",
            "sequence_number": 2,
            "item_id": "call_vertical",
            "output_index": 1,
            "name": "filesystem.apply_text_patch",
            "arguments": arguments
        }),
        serde_json::json!({
            "type": "response.completed",
            "sequence_number": 3,
            "response": {"id": "resp_vertical_slice"}
        }),
    ];
    let body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .expect("write provider response");
}
