use std::time::Duration;

use lunascope_core::{CredentialReference, ProviderConfig, ProviderProtocol, ProviderType};
use lunascope_integrations::{
    KeyringCredentialStore, NativeProviderClient, ProviderInvocation, ProviderMessage,
    ProviderMessageRole, SecretValue,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

fn config(
    address: std::net::SocketAddr,
    provider_type: ProviderType,
    protocol: ProviderProtocol,
    credential_id: &str,
) -> ProviderConfig {
    ProviderConfig {
        id: format!("{provider_type:?}").to_lowercase(),
        provider_type,
        protocol,
        display_name: "Protocol fixture".into(),
        base_url: format!("http://{address}"),
        credential_reference_id: credential_id.into(),
        default_model_id: "fixture-model".into(),
        custom_headers: Vec::new(),
        context_window_tokens: Some(64_000),
        supports_tools: true,
        supports_vision: false,
        supports_structured_output: true,
        enabled: true,
    }
}

fn invocation() -> ProviderInvocation {
    ProviderInvocation {
        model: "fixture-model".into(),
        instructions: Some("Be concise".into()),
        messages: vec![ProviderMessage {
            role: ProviderMessageRole::User,
            content: serde_json::json!("Hello"),
        }],
        tools: Vec::new(),
        max_output_tokens: 128,
        thinking_enabled: None,
        reasoning_effort: None,
        reasoning_summary: None,
    }
}

async fn fixture_server(body: String) -> (std::net::SocketAddr, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut socket).await.expect("request");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        for chunk in response.as_bytes().chunks(29) {
            socket.write_all(chunk).await.expect("response chunk");
        }
        request
    });
    (address, task)
}

#[tokio::test]
async fn openai_responses_client_uses_keyring_and_never_persists_secret() {
    let credential_id = format!("test-{}", uuid::Uuid::new_v4());
    let secret_text = format!("canary-{}", uuid::Uuid::new_v4());
    let reference = CredentialReference {
        id: credential_id.clone(),
        provider: ProviderType::OpenAi,
        label: "Provider client test".into(),
    };
    let store = KeyringCredentialStore;
    let secret = SecretValue::new(secret_text.clone()).expect("secret");
    store.put(&reference, &secret).expect("store credential");

    let body = [
        serde_json::json!({"type":"response.created","sequence_number":1,"response":{"id":"resp_1"}}),
        serde_json::json!({"type":"response.output_text.delta","sequence_number":2,"delta":"Hello"}),
        serde_json::json!({"type":"response.completed","sequence_number":3,"response":{"id":"resp_1","usage":{"input_tokens":7,"output_tokens":2}}}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (address, server) = fixture_server(body).await;
    let outcome = async {
        let client = NativeProviderClient::from_keyring(
            &config(
                address,
                ProviderType::OpenAi,
                ProviderProtocol::OpenAiResponses,
                &credential_id,
            ),
            &store,
        )?;
        let mut request = invocation();
        request.reasoning_effort = Some("high".into());
        request.reasoning_summary = Some("auto".into());
        client
            .stream(
                &request,
                Duration::from_secs(5),
                CancellationToken::new(),
                |_| Ok(()),
            )
            .await
    }
    .await;
    let request = server.await.expect("server");
    let delete = store.delete(&reference);
    delete.expect("delete credential after request");
    let summary = outcome.expect("provider response");
    assert_eq!(summary.text, "Hello");
    assert_eq!(summary.usage.input_tokens, Some(7));
    assert!(request.contains("POST /v1/responses HTTP/1.1"));
    assert!(request.contains(&format!("authorization: Bearer {secret_text}")));
    assert!(request.contains("\"store\":false"));
    assert!(request.contains("\"reasoning\":{\"effort\":\"high\",\"summary\":\"auto\"}"));
}

#[tokio::test]
async fn deepseek_openai_compatible_client_normalizes_usage() {
    let body = concat!(
        "data: {\"id\":\"chat_1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Deep\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chat_1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Seek\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\n",
        "data: [DONE]\n\n"
    );
    let (address, server) = fixture_server(body.into()).await;
    let client = NativeProviderClient::from_secret(
        &config(
            address,
            ProviderType::DeepSeek,
            ProviderProtocol::OpenAiChatCompletions,
            "deepseek-test",
        ),
        SecretValue::new("deepseek-canary").unwrap(),
    )
    .unwrap();
    let mut direct_invocation = invocation();
    direct_invocation.thinking_enabled = Some(false);
    let summary = client
        .stream(
            &direct_invocation,
            Duration::from_secs(5),
            CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .unwrap();
    let request = server.await.unwrap();
    assert_eq!(summary.text, "DeepSeek");
    assert_eq!(summary.usage.output_tokens, Some(3));
    assert!(request.contains("POST /chat/completions HTTP/1.1"));
    assert!(request.contains("\"stream_options\":{\"include_usage\":true}"));
    assert!(request.contains("\"thinking\":{\"type\":\"disabled\"}"));
}

#[tokio::test]
#[ignore = "performs a real DeepSeek request using Windows Credential Manager"]
async fn deepseek_v4_flash_live_keyring_canary() {
    let config = ProviderConfig {
        id: "deepseek-primary".into(),
        provider_type: ProviderType::DeepSeek,
        protocol: ProviderProtocol::OpenAiChatCompletions,
        display_name: "DeepSeek official".into(),
        base_url: "https://api.deepseek.com".into(),
        credential_reference_id: "deepseek-primary".into(),
        default_model_id: "deepseek-v4-flash".into(),
        custom_headers: Vec::new(),
        context_window_tokens: None,
        supports_tools: true,
        supports_vision: false,
        supports_structured_output: true,
        enabled: true,
    };
    let client =
        NativeProviderClient::from_keyring(&config, &KeyringCredentialStore).expect("credential");
    let summary = client
        .stream(
            &ProviderInvocation {
                model: config.default_model_id.clone(),
                instructions: Some("Reply with exactly OK and no punctuation.".into()),
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: serde_json::json!("LunaScope native provider canary"),
                }],
                tools: Vec::new(),
                max_output_tokens: 128,
                thinking_enabled: Some(false),
                reasoning_effort: None,
                reasoning_summary: None,
            },
            Duration::from_secs(45),
            CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .expect("live DeepSeek response");
    eprintln!(
        "DeepSeek live canary result: response_id={}, stop_reason={:?}, input_tokens={:?}, output_tokens={:?}, text_length={}",
        summary.response_id,
        summary.stop_reason,
        summary.usage.input_tokens,
        summary.usage.output_tokens,
        summary.text.len()
    );
    assert_eq!(summary.text.trim(), "OK");
}

#[tokio::test]
async fn anthropic_client_sends_version_header_and_named_events() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":11}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Claude\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":4}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let (address, server) = fixture_server(body.into()).await;
    let client = NativeProviderClient::from_secret(
        &config(
            address,
            ProviderType::Anthropic,
            ProviderProtocol::AnthropicMessages,
            "anthropic-test",
        ),
        SecretValue::new("anthropic-canary").unwrap(),
    )
    .unwrap();
    let summary = client
        .stream(
            &invocation(),
            Duration::from_secs(5),
            CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .unwrap();
    let request = server.await.unwrap();
    assert_eq!(summary.text, "Claude");
    assert_eq!(summary.stop_reason.as_deref(), Some("end_turn"));
    assert!(request.contains("POST /v1/messages HTTP/1.1"));
    assert!(request.contains("x-api-key: anthropic-canary"));
    assert!(request.contains("anthropic-version: 2023-06-01"));
}

async fn read_request(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let header_end = loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
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
