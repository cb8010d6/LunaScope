use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_ERROR_BODY_BYTES: usize = 8192;
const MAX_SSE_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct OpenAiResponseRequest {
    pub model: String,
    pub input: Value,
    pub instructions: Option<String>,
    pub tools: Vec<Value>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderStreamEvent {
    TextDelta {
        sequence: u64,
        delta: String,
    },
    FunctionArgumentsDelta {
        sequence: u64,
        item_id: String,
        delta: String,
    },
    FunctionArgumentsDone {
        sequence: u64,
        item_id: String,
        name: String,
        arguments: Value,
    },
    Completed {
        sequence: Option<u64>,
        response_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderStreamSummary {
    pub response_id: String,
    pub text: String,
    pub event_count: u64,
    pub duration_ms: u64,
    pub function_calls: Vec<ProviderFunctionCall>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFunctionCall {
    pub item_id: String,
    pub name: String,
    pub arguments: Value,
}

/// A native OpenAI Responses API adapter.
///
/// The bearer credential is moved directly into a sensitive HTTP header and
/// is neither retained as a public field nor exposed through Debug output.
pub struct OpenAiResponsesProvider {
    client: Client,
    endpoint: Url,
}

impl OpenAiResponsesProvider {
    pub fn new(endpoint: Url, bearer_credential: &str) -> Result<Self, ProviderError> {
        validate_endpoint(&endpoint)?;
        if bearer_credential.trim().is_empty() {
            return Err(ProviderError::MissingCredential);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {bearer_credential}"))
            .map_err(|_| ProviderError::InvalidCredential)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let client = Client::builder().default_headers(headers).build()?;
        Ok(Self { client, endpoint })
    }

    pub async fn stream(
        &self,
        request: &OpenAiResponseRequest,
        timeout: Duration,
        cancellation: CancellationToken,
        mut consumer: impl FnMut(ProviderStreamEvent) -> Result<(), String>,
    ) -> Result<ProviderStreamSummary, ProviderError> {
        if timeout.is_zero() {
            return Err(ProviderError::InvalidTimeout);
        }
        let body = serde_json::json!({
            "model": request.model,
            "input": request.input,
            "instructions": request.instructions,
            "tools": request.tools,
            "max_output_tokens": request.max_output_tokens,
            "stream": true,
            "store": false,
        });
        let started = Instant::now();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            result = tokio::time::timeout(
                timeout,
                self.client.post(self.endpoint.clone()).json(&body).send(),
            ) => result.map_err(|_| ProviderError::Timeout)??,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let bytes = response.bytes().await?;
            let retained = &bytes[..bytes.len().min(MAX_ERROR_BODY_BYTES)];
            return Err(ProviderError::HttpStatus {
                status,
                body: String::from_utf8_lossy(retained).into_owned(),
            });
        }

        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        let mut text = String::new();
        let mut response_id = None;
        let mut last_sequence = None;
        let mut event_count = 0u64;
        let mut function_calls = Vec::new();
        loop {
            let next = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                result = tokio::time::timeout(timeout.saturating_sub(started.elapsed()), stream.next()) => {
                    result.map_err(|_| ProviderError::Timeout)?
                }
            };
            let Some(chunk) = next else {
                break;
            };
            for data in decoder.push(&chunk?)? {
                if data == "[DONE]" {
                    continue;
                }
                let value: Value = serde_json::from_str(&data)?;
                let event_type = value
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or(ProviderError::MissingEventType)?;
                let sequence = value.get("sequence_number").and_then(Value::as_u64);
                if let Some(sequence) = sequence {
                    if last_sequence.is_some_and(|previous| sequence <= previous) {
                        return Err(ProviderError::NonMonotonicSequence {
                            previous: last_sequence.expect("checked above"),
                            actual: sequence,
                        });
                    }
                    last_sequence = Some(sequence);
                }

                let event = match event_type {
                    "response.output_text.delta" => {
                        let sequence = sequence.ok_or(ProviderError::MissingSequence)?;
                        let delta = required_string(&value, "delta")?.to_owned();
                        text.push_str(&delta);
                        Some(ProviderStreamEvent::TextDelta { sequence, delta })
                    }
                    "response.function_call_arguments.delta" => {
                        Some(ProviderStreamEvent::FunctionArgumentsDelta {
                            sequence: sequence.ok_or(ProviderError::MissingSequence)?,
                            item_id: required_string(&value, "item_id")?.to_owned(),
                            delta: required_string(&value, "delta")?.to_owned(),
                        })
                    }
                    "response.function_call_arguments.done" => {
                        let sequence = sequence.ok_or(ProviderError::MissingSequence)?;
                        let item_id = required_string(&value, "item_id")?.to_owned();
                        let name = required_string(&value, "name")?.to_owned();
                        let arguments =
                            serde_json::from_str::<Value>(required_string(&value, "arguments")?)?;
                        function_calls.push(ProviderFunctionCall {
                            item_id: item_id.clone(),
                            name: name.clone(),
                            arguments: arguments.clone(),
                        });
                        Some(ProviderStreamEvent::FunctionArgumentsDone {
                            sequence,
                            item_id,
                            name,
                            arguments,
                        })
                    }
                    "response.completed" => {
                        let id = value
                            .get("response")
                            .and_then(|response| response.get("id"))
                            .and_then(Value::as_str)
                            .ok_or(ProviderError::MissingResponseId)?
                            .to_owned();
                        response_id = Some(id.clone());
                        Some(ProviderStreamEvent::Completed {
                            sequence,
                            response_id: id,
                        })
                    }
                    "response.failed" | "response.incomplete" | "error" => {
                        return Err(ProviderError::RemoteFailure(remote_error_message(&value)));
                    }
                    _ => None,
                };
                if let Some(event) = event {
                    event_count += 1;
                    consumer(event).map_err(ProviderError::Consumer)?;
                }
            }
        }
        decoder.finish()?;
        let response_id = response_id.ok_or(ProviderError::MissingCompletion)?;
        Ok(ProviderStreamSummary {
            response_id,
            text,
            event_count,
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            function_calls,
        })
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<(), ProviderError> {
    if endpoint.scheme() == "https" {
        return Ok(());
    }
    let local_http = endpoint.scheme() == "http"
        && endpoint.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    if local_http {
        Ok(())
    } else {
        Err(ProviderError::InsecureEndpoint(endpoint.clone()))
    }
}

fn required_string<'a>(value: &'a Value, key: &'static str) -> Result<&'a str, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ProviderError::MissingField(key))
}

fn remote_error_message(value: &Value) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("provider reported an unspecified failure")
        .to_owned()
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_SSE_BUFFER_BYTES {
            return Err(ProviderError::SseBufferLimit);
        }
        let mut frames = Vec::new();
        while let Some((end, delimiter_len)) = frame_boundary(&self.buffer) {
            let frame = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            let frame = std::str::from_utf8(&frame)?;
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                frames.push(data);
            }
        }
        Ok(frames)
    }

    fn finish(&self) -> Result<(), ProviderError> {
        if self.buffer.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(ProviderError::TruncatedSse)
        }
    }
}

fn frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error("provider endpoint must use HTTPS (loopback HTTP is allowed for tests): {0}")]
    InsecureEndpoint(Url),
    #[error("provider credential is required")]
    MissingCredential,
    #[error("provider credential cannot be represented as an HTTP header")]
    InvalidCredential,
    #[error("provider timeout must be greater than zero")]
    InvalidTimeout,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider request was cancelled")]
    Cancelled,
    #[error("provider returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("provider stream event is missing type")]
    MissingEventType,
    #[error("provider stream event is missing sequence_number")]
    MissingSequence,
    #[error("provider stream event is missing response id")]
    MissingResponseId,
    #[error("provider stream event is missing {0}")]
    MissingField(&'static str),
    #[error("provider event sequence is not monotonic: {previous} then {actual}")]
    NonMonotonicSequence { previous: u64, actual: u64 },
    #[error("provider stream failed: {0}")]
    RemoteFailure(String),
    #[error("provider stream ended without response.completed")]
    MissingCompletion,
    #[error("provider SSE frame exceeded the buffer limit")]
    SseBufferLimit,
    #[error("provider SSE stream ended with a partial frame")]
    TruncatedSse,
    #[error("stream consumer failed: {0}")]
    Consumer(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_across_arbitrary_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"data: {\"type\":\"response.")
                .unwrap()
                .is_empty()
        );
        let frames = decoder
            .push(b"created\"}\r\n\r\ndata: [DONE]\n\n")
            .expect("decode");
        assert_eq!(frames, vec!["{\"type\":\"response.created\"}", "[DONE]"]);
        decoder.finish().expect("complete");
    }

    #[test]
    fn rejects_non_local_plain_http() {
        assert!(matches!(
            validate_endpoint(&Url::parse("http://example.com/v1/responses").unwrap()),
            Err(ProviderError::InsecureEndpoint(_))
        ));
    }
}
