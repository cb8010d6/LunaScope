use std::{io, sync::Arc, time::Duration};

use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, ProjectId, RunId, RunState,
    ThreadId,
};
use lunascope_runtime::{
    DurableApprovalRuntime, OpenAiResponseRequest, OpenAiResponsesProvider, ProviderStreamEvent,
    RunIdentity,
};
use lunascope_storage::SqliteEventStore;
use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

const TEST_CREDENTIAL: &str = "test-credential-must-not-persist";

fn identity() -> RunIdentity {
    RunIdentity {
        project_id: ProjectId::from("project-provider"),
        thread_id: ThreadId::from("thread-provider"),
        run_id: RunId::from("run-provider"),
        correlation_id: CorrelationId::from("correlation-provider"),
    }
}

fn event(id: &str, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(id),
        0,
        "2026-07-27T10:00:00Z",
        ProjectId::from("project-provider"),
        ThreadId::from("thread-provider"),
        RunId::from("run-provider"),
        CorrelationId::from("correlation-provider"),
        EventSource::System,
        payload,
    )
}

#[tokio::test]
async fn streams_official_response_events_into_durable_model_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let request = read_http_request(&mut socket).await.expect("read request");
        assert!(request.contains("POST /v1/responses HTTP/1.1"));
        assert!(request.contains(&format!("authorization: Bearer {TEST_CREDENTIAL}")));
        assert!(request.contains("\"stream\":true"));
        assert!(request.contains("\"store\":false"));
        assert!(request.contains("\"model\":\"test-model\""));

        let body = concat!(
            "data: {\"type\":\"response.created\",\"sequence_number\":1,\"response\":{\"id\":\"resp_test\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":2,\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"Patch \"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":3,\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"ready\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"sequence_number\":4,\"item_id\":\"call_1\",\"output_index\":1,\"delta\":\"{\\\"path\\\":\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"sequence_number\":5,\"item_id\":\"call_1\",\"output_index\":1,\"name\":\"filesystem.apply_text_patch\",\"arguments\":\"{\\\"path\\\":\\\"src/math.ps1\\\"}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"sequence_number\":6,\"response\":{\"id\":\"resp_test\"}}\n\n"
        );
        let headers = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(headers.as_bytes()).await.expect("headers");
        for chunk in body.as_bytes().chunks(37) {
            socket.write_all(chunk).await.expect("stream chunk");
            socket.flush().await.expect("flush");
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
    });

    let directory = tempfile::tempdir().expect("temp directory");
    let store =
        Arc::new(SqliteEventStore::open(directory.path().join("events.db")).expect("event store"));
    store
        .append_batch_next(vec![
            event(
                "created",
                EventData::RunCreated {
                    title: "Provider streaming".into(),
                    initial_prompt: "stream".into(),
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
                    reason: "invoke provider".into(),
                },
            ),
        ])
        .expect("initialize run");
    let runtime = DurableApprovalRuntime::new(store.clone(), identity());
    let provider = OpenAiResponsesProvider::new(
        Url::parse(&format!("http://{address}/v1/responses")).expect("endpoint"),
        TEST_CREDENTIAL,
    )
    .expect("provider");
    let summary = provider
        .stream(
            &OpenAiResponseRequest {
                model: "test-model".into(),
                input: serde_json::json!("Describe the intended patch"),
                instructions: Some("Return a compact plan.".into()),
                tools: Vec::new(),
                max_output_tokens: Some(128),
            },
            Duration::from_secs(5),
            CancellationToken::new(),
            |stream_event| {
                if let ProviderStreamEvent::TextDelta { sequence, delta } = stream_event {
                    runtime
                        .record_model_chunk("resp_test", sequence, delta)
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            },
        )
        .await
        .expect("provider stream");
    server.await.expect("server");
    assert_eq!(summary.response_id, "resp_test");
    assert_eq!(summary.text, "Patch ready");
    assert_eq!(summary.event_count, 5);
    assert_eq!(summary.function_calls.len(), 1);
    assert_eq!(
        summary.function_calls[0].name,
        "filesystem.apply_text_patch"
    );
    assert_eq!(
        summary.function_calls[0].arguments,
        serde_json::json!({"path": "src/math.ps1"})
    );

    let events = store
        .events_after(&RunId::from("run-provider"), 0)
        .expect("events");
    let chunks = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventData::ModelStreamChunk { index, text, .. } => Some((*index, text.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(chunks, vec![(2, "Patch "), (3, "ready")]);
    let serialized = serde_json::to_string(&events).expect("serialize events");
    assert!(!serialized.contains(TEST_CREDENTIAL));
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    let header_end;
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
