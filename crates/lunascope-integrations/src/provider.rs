use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use lunascope_core::{
    CredentialReference, ProviderConfig, ProviderHeaderValue, ProviderProtocol, ProviderType,
};
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    CredentialError, KeyringCredentialStore, NormalizedProviderEvent, NormalizedStreamSummary,
    NormalizedToolCall, ProtocolNormalizer, SecretValue, SseDecoder, StreamProtocolError,
};

const MAX_ERROR_BODY_BYTES: usize = 8192;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMessageRole {
    User,
    Assistant,
    AssistantToolCall,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: ProviderMessageRole,
    pub content: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAttachment {
    pub name: String,
    pub media_type: String,
    pub data_base64: String,
    pub is_document: bool,
}

impl ProviderMessage {
    pub fn user_with_attachments(
        text: impl Into<String>,
        attachments: Vec<ProviderAttachment>,
    ) -> Self {
        Self {
            role: ProviderMessageRole::User,
            content: serde_json::json!({
                "lunaScopeText": text.into(),
                "lunaScopeAttachments": attachments,
            }),
        }
    }

    pub fn assistant_tool_calls(text: impl Into<String>, calls: &[NormalizedToolCall]) -> Self {
        Self {
            role: ProviderMessageRole::AssistantToolCall,
            content: serde_json::json!({
                "text": text.into(),
                "toolCalls": calls.iter().map(|call| serde_json::json!({
                    "callId": call.item_id,
                    "name": call.name,
                    "arguments": call.arguments,
                })).collect::<Vec<_>>(),
            }),
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        success: bool,
        output: Value,
    ) -> Self {
        Self {
            role: ProviderMessageRole::Tool,
            content: serde_json::json!({
                "callId": call_id.into(),
                "name": name.into(),
                "success": success,
                "output": output,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderInvocation {
    pub model: String,
    pub instructions: Option<String>,
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<ProviderToolDefinition>,
    pub max_output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_enabled: Option<bool>,
    /// Provider-native reasoning effort, when the selected protocol supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Requests a user-safe Provider reasoning summary. Raw chain-of-thought is
    /// never requested or normalized into LunaScope's public event stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,
}

pub struct NativeProviderClient {
    client: Client,
    endpoint: Url,
    protocol: ProviderProtocol,
}

impl NativeProviderClient {
    pub fn from_keyring(
        config: &ProviderConfig,
        credentials: &KeyringCredentialStore,
    ) -> Result<Self, ProviderClientError> {
        Self::build(config, |id| {
            credentials
                .get(&CredentialReference {
                    id: id.to_owned(),
                    provider: config.provider_type,
                    label: String::new(),
                })
                .map_err(ProviderClientError::from)
        })
    }

    pub fn from_secret(
        config: &ProviderConfig,
        secret: SecretValue,
    ) -> Result<Self, ProviderClientError> {
        let primary_id = config.credential_reference_id.clone();
        let mut secret = Some(secret);
        Self::build(config, move |id| {
            if id != primary_id {
                return Err(ProviderClientError::MissingCredentialReference(
                    id.to_owned(),
                ));
            }
            secret
                .take()
                .ok_or_else(|| ProviderClientError::MissingCredentialReference(id.to_owned()))
        })
    }

    fn build(
        config: &ProviderConfig,
        mut resolve: impl FnMut(&str) -> Result<SecretValue, ProviderClientError>,
    ) -> Result<Self, ProviderClientError> {
        validate_provider_config(config)?;
        if !config.enabled {
            return Err(ProviderClientError::ProviderDisabled);
        }
        let endpoint = endpoint_url(config)?;
        let primary = resolve(&config.credential_reference_id)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        match config.protocol {
            ProviderProtocol::OpenAiResponses | ProviderProtocol::OpenAiChatCompletions => {
                let mut value = HeaderValue::from_str(&format!("Bearer {}", primary.expose()))
                    .map_err(|_| ProviderClientError::InvalidHeaderValue("authorization".into()))?;
                value.set_sensitive(true);
                headers.insert(AUTHORIZATION, value);
            }
            ProviderProtocol::AnthropicMessages => {
                insert_sensitive(&mut headers, "x-api-key", primary.expose())?;
                headers.insert(
                    HeaderName::from_static("anthropic-version"),
                    HeaderValue::from_static(ANTHROPIC_VERSION),
                );
            }
        }
        for header in &config.custom_headers {
            let name = HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| ProviderClientError::InvalidHeaderName(header.name.clone()))?;
            let (raw, sensitive) = match &header.value {
                ProviderHeaderValue::Literal(value) => {
                    if is_sensitive_header(&header.name) {
                        return Err(ProviderClientError::SensitiveLiteralHeader(
                            header.name.clone(),
                        ));
                    }
                    (value.clone(), false)
                }
                ProviderHeaderValue::CredentialReference(id) => {
                    (resolve(id)?.expose().to_owned(), true)
                }
            };
            let mut value = HeaderValue::from_str(&raw)
                .map_err(|_| ProviderClientError::InvalidHeaderValue(header.name.clone()))?;
            value.set_sensitive(sensitive);
            headers.insert(name, value);
        }
        let client = Client::builder().default_headers(headers).build()?;
        Ok(Self {
            client,
            endpoint,
            protocol: config.protocol,
        })
    }

    pub async fn stream(
        &self,
        invocation: &ProviderInvocation,
        timeout: Duration,
        cancellation: CancellationToken,
        mut consumer: impl FnMut(NormalizedProviderEvent) -> Result<(), String>,
    ) -> Result<NormalizedStreamSummary, ProviderClientError> {
        if timeout.is_zero() {
            return Err(ProviderClientError::InvalidTimeout);
        }
        validate_invocation(invocation)?;
        let body = request_body(self.protocol, invocation);
        let started = Instant::now();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderClientError::Cancelled),
            result = tokio::time::timeout(
                timeout,
                self.client.post(self.endpoint.clone()).json(&body).send(),
            ) => result.map_err(|_| ProviderClientError::Timeout)??,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let remaining = timeout.saturating_sub(started.elapsed());
            let bytes = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderClientError::Cancelled),
                result = tokio::time::timeout(remaining, response.bytes()) => {
                    result.map_err(|_| ProviderClientError::Timeout)??
                }
            };
            let retained = &bytes[..bytes.len().min(MAX_ERROR_BODY_BYTES)];
            return Err(ProviderClientError::HttpStatus {
                status,
                body: String::from_utf8_lossy(retained).into_owned(),
            });
        }

        let mut decoder = SseDecoder::default();
        let mut normalizer = ProtocolNormalizer::new(self.protocol);
        let mut stream = response.bytes_stream();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            let next = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderClientError::Cancelled),
                result = tokio::time::timeout(remaining, stream.next()) => {
                    result.map_err(|_| ProviderClientError::Timeout)?
                }
            };
            let Some(chunk) = next else {
                break;
            };
            for frame in decoder.push(&chunk?)? {
                for event in normalizer.ingest(frame)? {
                    consumer(event).map_err(ProviderClientError::Consumer)?;
                }
            }
        }
        decoder.finish()?;
        normalizer.finish().map_err(ProviderClientError::from)
    }
}

pub fn validate_provider_config(config: &ProviderConfig) -> Result<(), ProviderClientError> {
    if config.id.trim().is_empty()
        || config.credential_reference_id.trim().is_empty()
        || config.default_model_id.trim().is_empty()
        || config.context_window_tokens == Some(0)
    {
        return Err(ProviderClientError::InvalidConfig);
    }
    let protocol_allowed = match config.provider_type {
        ProviderType::OpenAi => config.protocol == ProviderProtocol::OpenAiResponses,
        ProviderType::Anthropic => config.protocol == ProviderProtocol::AnthropicMessages,
        ProviderType::DeepSeek => matches!(
            config.protocol,
            ProviderProtocol::OpenAiChatCompletions | ProviderProtocol::AnthropicMessages
        ),
        ProviderType::GenericOpenAiCompatible => {
            config.protocol == ProviderProtocol::OpenAiChatCompletions
        }
        ProviderType::GenericAnthropicCompatible => {
            config.protocol == ProviderProtocol::AnthropicMessages
        }
    };
    if !protocol_allowed {
        return Err(ProviderClientError::ProtocolMismatch);
    }
    let _ = endpoint_url(config)?;
    for header in &config.custom_headers {
        HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|_| ProviderClientError::InvalidHeaderName(header.name.clone()))?;
        match &header.value {
            ProviderHeaderValue::Literal(value) => {
                if is_sensitive_header(&header.name) {
                    return Err(ProviderClientError::SensitiveLiteralHeader(
                        header.name.clone(),
                    ));
                }
                HeaderValue::from_str(value)
                    .map_err(|_| ProviderClientError::InvalidHeaderValue(header.name.clone()))?;
            }
            ProviderHeaderValue::CredentialReference(id) if id.trim().is_empty() => {
                return Err(ProviderClientError::MissingCredentialReference(id.clone()));
            }
            ProviderHeaderValue::CredentialReference(_) => {}
        }
    }
    Ok(())
}

fn endpoint_url(config: &ProviderConfig) -> Result<Url, ProviderClientError> {
    let mut base = Url::parse(&config.base_url)
        .map_err(|error| ProviderClientError::InvalidUrl(error.to_string()))?;
    validate_endpoint(&base)?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let path = base.path().trim_end_matches('/');
    let suffix = match config.protocol {
        ProviderProtocol::OpenAiResponses if path.ends_with("/v1") => "responses",
        ProviderProtocol::OpenAiResponses => "v1/responses",
        ProviderProtocol::OpenAiChatCompletions => "chat/completions",
        ProviderProtocol::AnthropicMessages if path.ends_with("/v1") => "messages",
        ProviderProtocol::AnthropicMessages => "v1/messages",
    };
    base.join(suffix)
        .map_err(|error| ProviderClientError::InvalidUrl(error.to_string()))
}

fn validate_endpoint(endpoint: &Url) -> Result<(), ProviderClientError> {
    let safe = endpoint.scheme() == "https"
        || (endpoint.scheme() == "http"
            && endpoint.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<IpAddr>()
                        .is_ok_and(|address| address.is_loopback())
            }));
    if safe {
        Ok(())
    } else {
        Err(ProviderClientError::InsecureEndpoint(endpoint.clone()))
    }
}

fn validate_invocation(invocation: &ProviderInvocation) -> Result<(), ProviderClientError> {
    if invocation.model.trim().is_empty()
        || invocation.messages.is_empty()
        || invocation.max_output_tokens == 0
    {
        return Err(ProviderClientError::InvalidInvocation);
    }
    Ok(())
}

fn request_body(protocol: ProviderProtocol, invocation: &ProviderInvocation) -> Value {
    let messages = request_messages(protocol, &invocation.messages);
    match protocol {
        ProviderProtocol::OpenAiResponses => {
            let mut body = serde_json::json!({
                "model": invocation.model,
                "instructions": invocation.instructions,
                "input": messages,
                "tools": invocation.tools.iter().map(|tool| serde_json::json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    "strict": tool.strict,
                })).collect::<Vec<_>>(),
                "max_output_tokens": invocation.max_output_tokens,
                "stream": true,
                "store": false,
            });
            if invocation.reasoning_effort.is_some() || invocation.reasoning_summary.is_some() {
                body["reasoning"] = serde_json::json!({
                    "effort": invocation.reasoning_effort,
                    "summary": invocation.reasoning_summary,
                });
            }
            body
        }
        ProviderProtocol::OpenAiChatCompletions => {
            let mut messages = messages;
            if let Some(instructions) = &invocation.instructions {
                messages.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": instructions,
                    }),
                );
            }
            let mut body = serde_json::json!({
                "model": invocation.model,
                "messages": messages,
                "tools": invocation.tools.iter().map(|tool| serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                        "strict": tool.strict,
                    }
                })).collect::<Vec<_>>(),
                "max_tokens": invocation.max_output_tokens,
                "stream": true,
                "stream_options": {"include_usage": true},
            });
            if let Some(enabled) = invocation.thinking_enabled {
                body["thinking"] = serde_json::json!({
                    "type": if enabled { "enabled" } else { "disabled" }
                });
            }
            body
        }
        ProviderProtocol::AnthropicMessages => serde_json::json!({
            "model": invocation.model,
            "system": invocation.instructions,
            "messages": messages,
            "tools": invocation.tools.iter().map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })).collect::<Vec<_>>(),
            "max_tokens": invocation.max_output_tokens,
            "stream": true,
        }),
    }
}

fn request_messages(protocol: ProviderProtocol, messages: &[ProviderMessage]) -> Vec<Value> {
    let mut result = Vec::new();
    for message in messages {
        match message.role {
            ProviderMessageRole::User => {
                result.push(serde_json::json!({
                    "role": "user",
                    "content": provider_user_content(protocol, &message.content),
                }));
            }
            ProviderMessageRole::Assistant => {
                result.push(serde_json::json!({
                    "role": "assistant",
                    "content": message.content,
                }));
            }
            ProviderMessageRole::AssistantToolCall => {
                let text = message
                    .content
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let calls = message
                    .content
                    .get("toolCalls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                match protocol {
                    ProviderProtocol::OpenAiResponses => {
                        if !text.trim().is_empty() {
                            result.push(serde_json::json!({
                                "role": "assistant",
                                "content": text,
                            }));
                        }
                        for call in calls {
                            result.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": call.get("callId").and_then(Value::as_str).unwrap_or_default(),
                                "name": call.get("name").and_then(Value::as_str).unwrap_or_default(),
                                "arguments": serde_json::to_string(
                                    call.get("arguments").unwrap_or(&Value::Null)
                                ).unwrap_or_else(|_| "{}".into()),
                            }));
                        }
                    }
                    ProviderProtocol::OpenAiChatCompletions => {
                        result.push(serde_json::json!({
                            "role": "assistant",
                            "content": if text.trim().is_empty() { Value::Null } else { Value::String(text.to_owned()) },
                            "tool_calls": calls.iter().map(|call| serde_json::json!({
                                "id": call.get("callId").and_then(Value::as_str).unwrap_or_default(),
                                "type": "function",
                                "function": {
                                    "name": call.get("name").and_then(Value::as_str).unwrap_or_default(),
                                    "arguments": serde_json::to_string(
                                        call.get("arguments").unwrap_or(&Value::Null)
                                    ).unwrap_or_else(|_| "{}".into()),
                                }
                            })).collect::<Vec<_>>(),
                        }));
                    }
                    ProviderProtocol::AnthropicMessages => {
                        let mut content = Vec::new();
                        if !text.trim().is_empty() {
                            content.push(serde_json::json!({"type": "text", "text": text}));
                        }
                        content.extend(calls.iter().map(|call| serde_json::json!({
                            "type": "tool_use",
                            "id": call.get("callId").and_then(Value::as_str).unwrap_or_default(),
                            "name": call.get("name").and_then(Value::as_str).unwrap_or_default(),
                            "input": call.get("arguments").cloned().unwrap_or(Value::Null),
                        })));
                        result.push(serde_json::json!({
                            "role": "assistant",
                            "content": content,
                        }));
                    }
                }
            }
            ProviderMessageRole::Tool => {
                let call_id = message
                    .content
                    .get("callId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let success = message
                    .content
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let output = message
                    .content
                    .get("output")
                    .cloned()
                    .unwrap_or(Value::Null);
                let output_text = match &output {
                    Value::String(text) => text.clone(),
                    _ => serde_json::to_string(&output).unwrap_or_else(|_| "null".into()),
                };
                match protocol {
                    ProviderProtocol::OpenAiResponses => {
                        result.push(serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": output_text,
                        }));
                    }
                    ProviderProtocol::OpenAiChatCompletions => {
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output_text,
                        }));
                    }
                    ProviderProtocol::AnthropicMessages => {
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "tool_use_id": call_id,
                                "content": output_text,
                                "is_error": !success,
                            }],
                        }));
                    }
                }
            }
        }
    }
    result
}

fn provider_user_content(protocol: ProviderProtocol, content: &Value) -> Value {
    let Some(text) = content.get("lunaScopeText").and_then(Value::as_str) else {
        return content.clone();
    };
    let attachments = content
        .get("lunaScopeAttachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match protocol {
        ProviderProtocol::OpenAiResponses => {
            let mut blocks = vec![serde_json::json!({"type": "input_text", "text": text})];
            for attachment in attachments {
                let media_type = attachment
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let data = attachment
                    .get("dataBase64")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_document = attachment
                    .get("isDocument")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_document {
                    blocks.push(serde_json::json!({
                        "type": "input_file",
                        "filename": attachment.get("name").and_then(Value::as_str).unwrap_or("document.pdf"),
                        "file_data": format!("data:{media_type};base64,{data}"),
                    }));
                } else if media_type.starts_with("image/") {
                    blocks.push(serde_json::json!({
                        "type": "input_image",
                        "image_url": format!("data:{media_type};base64,{data}"),
                    }));
                }
            }
            Value::Array(blocks)
        }
        ProviderProtocol::OpenAiChatCompletions => {
            let mut blocks = vec![serde_json::json!({"type": "text", "text": text})];
            for attachment in attachments {
                let media_type = attachment
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let data = attachment
                    .get("dataBase64")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if media_type.starts_with("image/") {
                    blocks.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": {"url": format!("data:{media_type};base64,{data}")},
                    }));
                }
            }
            Value::Array(blocks)
        }
        ProviderProtocol::AnthropicMessages => {
            let mut blocks = vec![serde_json::json!({"type": "text", "text": text})];
            for attachment in attachments {
                let media_type = attachment
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let data = attachment
                    .get("dataBase64")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_document = attachment
                    .get("isDocument")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_document || media_type.starts_with("image/") {
                    blocks.push(serde_json::json!({
                        "type": if is_document { "document" } else { "image" },
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data,
                        },
                    }));
                }
            }
            Value::Array(blocks)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_turn() -> Vec<ProviderMessage> {
        let calls = vec![NormalizedToolCall {
            item_id: "call-1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "README.md"}),
        }];
        vec![
            ProviderMessage::assistant_tool_calls("", &calls),
            ProviderMessage::tool_result(
                "call-1",
                "read_file",
                true,
                Value::String("contents".into()),
            ),
        ]
    }

    #[test]
    fn chat_completions_preserves_native_tool_call_linkage() {
        let messages = request_messages(ProviderProtocol::OpenAiChatCompletions, &tool_turn());
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call-1");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call-1");
    }

    #[test]
    fn responses_preserves_native_function_call_linkage() {
        let messages = request_messages(ProviderProtocol::OpenAiResponses, &tool_turn());
        assert_eq!(messages[0]["type"], "function_call");
        assert_eq!(messages[0]["call_id"], "call-1");
        assert_eq!(messages[1]["type"], "function_call_output");
        assert_eq!(messages[1]["call_id"], "call-1");
    }

    #[test]
    fn anthropic_preserves_native_tool_use_linkage() {
        let messages = request_messages(ProviderProtocol::AnthropicMessages, &tool_turn());
        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[0]["content"][0]["id"], "call-1");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
        assert_eq!(messages[1]["content"][0]["tool_use_id"], "call-1");
    }

    #[test]
    fn multimodal_user_content_is_protocol_native() {
        let message = ProviderMessage::user_with_attachments(
            "inspect",
            vec![ProviderAttachment {
                name: "figure.png".into(),
                media_type: "image/png".into(),
                data_base64: "YWJj".into(),
                is_document: false,
            }],
        );
        let responses = request_messages(
            ProviderProtocol::OpenAiResponses,
            std::slice::from_ref(&message),
        );
        assert_eq!(responses[0]["content"][1]["type"], "input_image");
        let chat = request_messages(
            ProviderProtocol::OpenAiChatCompletions,
            std::slice::from_ref(&message),
        );
        assert_eq!(chat[0]["content"][1]["type"], "image_url");
        let anthropic = request_messages(ProviderProtocol::AnthropicMessages, &[message]);
        assert_eq!(anthropic[0]["content"][1]["type"], "image");
    }
}

fn insert_sensitive(
    headers: &mut HeaderMap,
    name: &'static str,
    raw: &str,
) -> Result<(), ProviderClientError> {
    let mut value = HeaderValue::from_str(raw)
        .map_err(|_| ProviderClientError::InvalidHeaderValue(name.into()))?;
    value.set_sensitive(true);
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "x-api-key" | "api-key" | "cookie" | "set-cookie"
    )
}

#[derive(Debug, Error)]
pub enum ProviderClientError {
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("provider URL is invalid: {0}")]
    InvalidUrl(String),
    #[error(transparent)]
    Protocol(#[from] StreamProtocolError),
    #[error("provider is disabled")]
    ProviderDisabled,
    #[error("provider configuration is invalid")]
    InvalidConfig,
    #[error("provider type and protocol do not match")]
    ProtocolMismatch,
    #[error("provider endpoint must use HTTPS, except loopback tests: {0}")]
    InsecureEndpoint(Url),
    #[error("provider invocation is invalid")]
    InvalidInvocation,
    #[error("provider timeout must be greater than zero")]
    InvalidTimeout,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider request was cancelled")]
    Cancelled,
    #[error("provider returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("provider header name is invalid: {0}")]
    InvalidHeaderName(String),
    #[error("provider header value is invalid: {0}")]
    InvalidHeaderValue(String),
    #[error("sensitive provider header must use a credential reference: {0}")]
    SensitiveLiteralHeader(String),
    #[error("credential reference is unavailable: {0}")]
    MissingCredentialReference(String),
    #[error("provider stream consumer failed: {0}")]
    Consumer(String),
}
