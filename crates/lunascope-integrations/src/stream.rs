use std::collections::BTreeMap;

use lunascope_core::ProviderProtocol;
use serde_json::Value;
use thiserror::Error;

const MAX_SSE_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>, StreamProtocolError> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_SSE_BUFFER_BYTES {
            return Err(StreamProtocolError::BufferLimit);
        }
        let mut frames = Vec::new();
        while let Some((end, delimiter_len)) = frame_boundary(&self.buffer) {
            let frame = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            let frame = std::str::from_utf8(&frame)?;
            let event = frame.lines().find_map(|line| {
                line.strip_prefix("event:")
                    .map(str::trim)
                    .map(str::to_owned)
            });
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                frames.push(SseFrame { event, data });
            }
        }
        Ok(frames)
    }

    pub fn finish(&self) -> Result<(), StreamProtocolError> {
        if self.buffer.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(StreamProtocolError::TruncatedFrame)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormalizedProviderEvent {
    TransportRetryScheduled {
        attempt: u32,
        maximum_retries: u32,
        delay_seconds: u64,
        reason: String,
    },
    TextDelta {
        sequence: u64,
        delta: String,
    },
    ReasoningSummaryDelta {
        sequence: u64,
        item_id: Option<String>,
        summary_index: u64,
        delta: String,
    },
    ToolInputDelta {
        sequence: u64,
        index: u64,
        item_id: Option<String>,
        name: Option<String>,
        delta: String,
    },
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
    },
    Completed {
        response_id: String,
        stop_reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedToolCall {
    pub item_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedStreamSummary {
    pub response_id: String,
    pub text: String,
    pub reasoning_summaries: Vec<String>,
    /// Provider-private reasoning required only for protocol-continuity replay.
    /// It is never emitted as a normalized event or persisted by LunaScope.
    pub private_reasoning: Option<String>,
    pub tool_calls: Vec<NormalizedToolCall>,
    pub usage: ProviderUsage,
    pub stop_reason: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct ToolBuffer {
    item_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

pub struct ProtocolNormalizer {
    protocol: ProviderProtocol,
    next_sequence: u64,
    last_remote_sequence: Option<u64>,
    response_id: Option<String>,
    text: String,
    reasoning_summaries: BTreeMap<u64, String>,
    private_reasoning: String,
    tools: BTreeMap<u64, ToolBuffer>,
    usage: ProviderUsage,
    stop_reason: Option<String>,
    completed: bool,
}

impl ProtocolNormalizer {
    pub fn new(protocol: ProviderProtocol) -> Self {
        Self {
            protocol,
            next_sequence: 1,
            last_remote_sequence: None,
            response_id: None,
            text: String::new(),
            reasoning_summaries: BTreeMap::new(),
            private_reasoning: String::new(),
            tools: BTreeMap::new(),
            usage: ProviderUsage::default(),
            stop_reason: None,
            completed: false,
        }
    }

    pub fn ingest(
        &mut self,
        frame: SseFrame,
    ) -> Result<Vec<NormalizedProviderEvent>, StreamProtocolError> {
        if frame.data == "[DONE]" {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(&frame.data)?;
        match self.protocol {
            ProviderProtocol::OpenAiResponses => self.openai_responses(value),
            ProviderProtocol::OpenAiChatCompletions => self.openai_chat(value),
            ProviderProtocol::AnthropicMessages => self.anthropic(frame.event.as_deref(), value),
        }
    }

    pub fn finish(self) -> Result<NormalizedStreamSummary, StreamProtocolError> {
        if !self.completed {
            return Err(StreamProtocolError::MissingCompletion);
        }
        let tool_calls = self
            .tools
            .into_values()
            .map(|tool| {
                let item_id = tool.item_id.ok_or(StreamProtocolError::MissingToolId)?;
                let name = tool.name.ok_or(StreamProtocolError::MissingToolName)?;
                let arguments = parse_tool_arguments(&tool.arguments)?;
                Ok(NormalizedToolCall {
                    item_id,
                    name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, StreamProtocolError>>()?;
        Ok(NormalizedStreamSummary {
            response_id: self
                .response_id
                .ok_or(StreamProtocolError::MissingResponseId)?,
            text: self.text,
            reasoning_summaries: self.reasoning_summaries.into_values().collect(),
            private_reasoning: (!self.private_reasoning.is_empty())
                .then_some(self.private_reasoning),
            tool_calls,
            usage: self.usage,
            stop_reason: self.stop_reason,
        })
    }

    fn openai_responses(
        &mut self,
        value: Value,
    ) -> Result<Vec<NormalizedProviderEvent>, StreamProtocolError> {
        let event_type = required_string(&value, "type")?;
        let sequence = value.get("sequence_number").and_then(Value::as_u64);
        if let Some(sequence) = sequence {
            if self
                .last_remote_sequence
                .is_some_and(|previous| sequence <= previous)
            {
                return Err(StreamProtocolError::NonMonotonicSequence);
            }
            self.last_remote_sequence = Some(sequence);
            self.next_sequence = sequence.saturating_add(1);
        }
        let sequence = sequence.unwrap_or_else(|| self.sequence());
        match event_type {
            "response.created" => {
                self.response_id = value
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Ok(Vec::new())
            }
            "response.output_text.delta" => {
                let delta = required_string(&value, "delta")?.to_owned();
                self.text.push_str(&delta);
                Ok(vec![NormalizedProviderEvent::TextDelta { sequence, delta }])
            }
            "response.reasoning_summary_text.delta" => {
                let summary_index = value
                    .get("summary_index")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                let delta = required_string(&value, "delta")?.to_owned();
                self.reasoning_summaries
                    .entry(summary_index)
                    .or_default()
                    .push_str(&delta);
                Ok(vec![NormalizedProviderEvent::ReasoningSummaryDelta {
                    sequence,
                    item_id: value
                        .get("item_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    summary_index,
                    delta,
                }])
            }
            "response.reasoning_summary_text.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done" => Ok(Vec::new()),
            "response.function_call_arguments.done" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .ok_or(StreamProtocolError::MissingField("output_index"))?;
                let tool = self.tools.entry(index).or_default();
                tool.item_id = Some(required_string(&value, "item_id")?.to_owned());
                tool.name = Some(required_string(&value, "name")?.to_owned());
                tool.arguments = required_string(&value, "arguments")?.to_owned();
                Ok(Vec::new())
            }
            "response.completed" => {
                self.completed = true;
                self.response_id = value
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.response_id.take());
                self.usage.input_tokens = value
                    .pointer("/response/usage/input_tokens")
                    .and_then(Value::as_u64);
                self.usage.output_tokens = value
                    .pointer("/response/usage/output_tokens")
                    .and_then(Value::as_u64);
                self.usage.cached_input_tokens = value
                    .pointer("/response/usage/input_tokens_details/cached_tokens")
                    .and_then(Value::as_u64);
                Ok(vec![self.completed_event()])
            }
            "response.failed" | "response.incomplete" | "error" => {
                Err(StreamProtocolError::RemoteFailure(error_message(&value)))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn openai_chat(
        &mut self,
        value: Value,
    ) -> Result<Vec<NormalizedProviderEvent>, StreamProtocolError> {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            self.response_id.get_or_insert_with(|| id.to_owned());
        }
        if value
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            return Ok(self.usage_event(&value).into_iter().collect());
        }
        let Some(choice) = value.pointer("/choices/0") else {
            return Err(StreamProtocolError::MissingField("choices[0]"));
        };
        let mut events = Vec::new();
        if let Some(delta) = choice.pointer("/delta/content").and_then(Value::as_str) {
            if !delta.is_empty() {
                self.text.push_str(delta);
                events.push(NormalizedProviderEvent::TextDelta {
                    sequence: self.sequence(),
                    delta: delta.to_owned(),
                });
            }
        }
        // Some OpenAI-compatible Providers require private reasoning replay when
        // a thinking turn contains tool calls. Capture it only for the next
        // protocol request; never emit it as a public normalized event.
        if let Some(delta) = choice
            .pointer("/delta/reasoning_content")
            .and_then(Value::as_str)
        {
            self.private_reasoning.push_str(delta);
        }
        // A separate reasoning_summary channel is safe for the user-facing UI.
        if let Some(delta) = choice
            .pointer("/delta/reasoning_summary")
            .and_then(Value::as_str)
            .filter(|delta| !delta.is_empty())
        {
            self.reasoning_summaries
                .entry(0)
                .or_default()
                .push_str(delta);
            events.push(NormalizedProviderEvent::ReasoningSummaryDelta {
                sequence: self.sequence(),
                item_id: None,
                summary_index: 0,
                delta: delta.to_owned(),
            });
        }
        if let Some(tool_calls) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for call in tool_calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or(StreamProtocolError::MissingField("tool_calls.index"))?;
                let delta = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (item_id, name) = {
                    let tool = self.tools.entry(index).or_default();
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        tool.item_id = Some(id.to_owned());
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        tool.name = Some(name.to_owned());
                    }
                    tool.arguments.push_str(delta);
                    (tool.item_id.clone(), tool.name.clone())
                };
                events.push(NormalizedProviderEvent::ToolInputDelta {
                    sequence: self.sequence(),
                    index,
                    item_id,
                    name,
                    delta: delta.to_owned(),
                });
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(reason.to_owned());
            self.completed = true;
            events.push(self.completed_event());
        }
        if let Some(usage) = self.usage_event(&value) {
            events.push(usage);
        }
        Ok(events)
    }

    fn anthropic(
        &mut self,
        event_name: Option<&str>,
        value: Value,
    ) -> Result<Vec<NormalizedProviderEvent>, StreamProtocolError> {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .or(event_name)
            .ok_or(StreamProtocolError::MissingField("type"))?;
        match event_type {
            "message_start" => {
                self.response_id = value
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.usage.input_tokens = value
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64);
                self.usage.cached_input_tokens = value
                    .pointer("/message/usage/cache_read_input_tokens")
                    .and_then(Value::as_u64);
                Ok(Vec::new())
            }
            "content_block_start" => {
                let index = required_u64(&value, "index")?;
                if value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
                {
                    let tool = self.tools.entry(index).or_default();
                    tool.item_id = value
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    tool.name = value
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Ok(Vec::new())
            }
            "content_block_delta" => {
                let index = required_u64(&value, "index")?;
                match value.pointer("/delta/type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let delta = value
                            .pointer("/delta/text")
                            .and_then(Value::as_str)
                            .ok_or(StreamProtocolError::MissingField("delta.text"))?
                            .to_owned();
                        self.text.push_str(&delta);
                        Ok(vec![NormalizedProviderEvent::TextDelta {
                            sequence: self.sequence(),
                            delta,
                        }])
                    }
                    Some("input_json_delta") => {
                        let delta = value
                            .pointer("/delta/partial_json")
                            .and_then(Value::as_str)
                            .ok_or(StreamProtocolError::MissingField("delta.partial_json"))?
                            .to_owned();
                        let (item_id, name) = {
                            let tool = self.tools.entry(index).or_default();
                            tool.arguments.push_str(&delta);
                            (tool.item_id.clone(), tool.name.clone())
                        };
                        Ok(vec![NormalizedProviderEvent::ToolInputDelta {
                            sequence: self.sequence(),
                            index,
                            item_id,
                            name,
                            delta,
                        }])
                    }
                    _ => Ok(Vec::new()),
                }
            }
            "message_delta" => {
                self.stop_reason = value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.usage.output_tokens = value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64);
                Ok(vec![NormalizedProviderEvent::Usage {
                    input_tokens: self.usage.input_tokens,
                    output_tokens: self.usage.output_tokens,
                    cached_input_tokens: self.usage.cached_input_tokens,
                }])
            }
            "message_stop" => {
                self.completed = true;
                Ok(vec![self.completed_event()])
            }
            "error" => Err(StreamProtocolError::RemoteFailure(error_message(&value))),
            _ => Ok(Vec::new()),
        }
    }

    fn sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    fn usage_event(&mut self, value: &Value) -> Option<NormalizedProviderEvent> {
        let usage = value.get("usage")?;
        self.usage.input_tokens = usage.get("prompt_tokens").and_then(Value::as_u64);
        self.usage.output_tokens = usage.get("completion_tokens").and_then(Value::as_u64);
        self.usage.cached_input_tokens = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64);
        Some(NormalizedProviderEvent::Usage {
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cached_input_tokens: self.usage.cached_input_tokens,
        })
    }

    fn completed_event(&self) -> NormalizedProviderEvent {
        NormalizedProviderEvent::Completed {
            response_id: self.response_id.clone().unwrap_or_default(),
            stop_reason: self.stop_reason.clone(),
        }
    }
}

fn parse_tool_arguments(value: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(value)
        .or_else(|_| serde_json::from_str(&repair_json_string_escapes(value)))
}

fn repair_json_string_escapes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    let mut in_string = false;
    while let Some(character) = characters.next() {
        if !in_string {
            match character {
                '"' => {
                    output.push(character);
                    in_string = true;
                }
                '\n' | '\r' | '\t' | ' ' => output.push(character),
                control if control.is_control() => {}
                _ => output.push(character),
            }
            continue;
        }
        match character {
            '"' => {
                output.push(character);
                in_string = false;
            }
            '\\' => {
                let next = characters.peek().copied();
                if next.is_some_and(|next| {
                    matches!(next, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u')
                }) {
                    output.push(character);
                    output.push(characters.next().expect("peeked JSON escape"));
                } else {
                    output.push('\\');
                    output.push('\\');
                }
            }
            '\n' => output.push_str("\\n"),
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                output.push_str("\\n");
            }
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            _ => output.push(character),
        }
    }
    output
}

fn required_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, StreamProtocolError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(StreamProtocolError::MissingField(field))
}

fn required_u64(value: &Value, field: &'static str) -> Result<u64, StreamProtocolError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(StreamProtocolError::MissingField(field))
}

fn error_message(value: &Value) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("provider reported an unspecified failure")
        .to_owned()
}

fn frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4));
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, 2));
    match (crlf, lf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

#[derive(Debug, Error)]
pub enum StreamProtocolError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error("provider SSE buffer exceeded one MiB")]
    BufferLimit,
    #[error("provider SSE stream ended with a partial frame")]
    TruncatedFrame,
    #[error("provider event is missing {0}")]
    MissingField(&'static str),
    #[error("provider response id is missing")]
    MissingResponseId,
    #[error("provider tool id is missing")]
    MissingToolId,
    #[error("provider tool name is missing")]
    MissingToolName,
    #[error("provider event sequence is not monotonic")]
    NonMonotonicSequence,
    #[error("provider stream ended without a completion event")]
    MissingCompletion,
    #[error("provider stream failed: {0}")]
    RemoteFailure(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: Option<&str>, data: Value) -> SseFrame {
        SseFrame {
            event: event.map(str::to_owned),
            data: data.to_string(),
        }
    }

    #[test]
    fn decodes_named_and_data_only_sse_across_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"event: content_block_").unwrap().is_empty());
        let frames = decoder
            .push(b"delta\r\ndata: {\"type\":\"content_block_delta\"}\r\n\r\n")
            .unwrap();
        assert_eq!(frames[0].event.as_deref(), Some("content_block_delta"));
        decoder.finish().unwrap();
    }

    #[test]
    fn normalizes_openai_chat_tool_and_usage() {
        let mut normalizer = ProtocolNormalizer::new(ProviderProtocol::OpenAiChatCompletions);
        normalizer
            .ingest(frame(
                None,
                serde_json::json!({
                    "id":"chat_1",
                    "choices":[{
                        "delta":{"content":"Hi","tool_calls":[{
                            "index":0,"id":"call_1","function":{"name":"patch","arguments":"{\"x\":"}
                        }]},
                        "finish_reason":null
                    }]
                }),
            ))
            .unwrap();
        normalizer
            .ingest(frame(
                None,
                serde_json::json!({
                    "id":"chat_1",
                    "choices":[{"delta":{"tool_calls":[{
                        "index":0,"function":{"arguments":"1}"}
                    }]},"finish_reason":"tool_calls"}],
                    "usage":{"prompt_tokens":10,"completion_tokens":4,"prompt_tokens_details":{"cached_tokens":6}}
                }),
            ))
            .unwrap();
        let summary = normalizer.finish().unwrap();
        assert_eq!(summary.text, "Hi");
        assert_eq!(summary.tool_calls[0].arguments, serde_json::json!({"x":1}));
        assert_eq!(summary.usage.input_tokens, Some(10));
        assert_eq!(summary.usage.cached_input_tokens, Some(6));
        assert_eq!(summary.stop_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn normalizes_provider_reasoning_summaries_without_raw_chain_content() {
        let mut responses = ProtocolNormalizer::new(ProviderProtocol::OpenAiResponses);
        for value in [
            serde_json::json!({"type":"response.created","sequence_number":1,"response":{"id":"resp_reasoning"}}),
            serde_json::json!({"type":"response.reasoning_summary_text.delta","sequence_number":2,"item_id":"rs_1","summary_index":0,"delta":"Inspecting the workspace"}),
            serde_json::json!({"type":"response.reasoning_summary_text.delta","sequence_number":3,"item_id":"rs_1","summary_index":0,"delta":" before editing."}),
            serde_json::json!({"type":"response.completed","sequence_number":4,"response":{"id":"resp_reasoning","usage":{"input_tokens":4,"output_tokens":8,"input_tokens_details":{"cached_tokens":3}}}}),
        ] {
            responses.ingest(frame(None, value)).unwrap();
        }
        let summary = responses.finish().unwrap();
        assert_eq!(
            summary.reasoning_summaries,
            vec!["Inspecting the workspace before editing."]
        );
        assert_eq!(summary.usage.cached_input_tokens, Some(3));

        let mut chat = ProtocolNormalizer::new(ProviderProtocol::OpenAiChatCompletions);
        chat.ingest(frame(
            None,
            serde_json::json!({
                "id":"chat_reasoning",
                "choices":[{
                    "delta":{
                        "reasoning_summary":"Checking requirements.",
                        "reasoning_content":"private raw chain"
                    },
                    "finish_reason":"stop"
                }]
            }),
        ))
        .unwrap();
        let summary = chat.finish().unwrap();
        assert_eq!(summary.reasoning_summaries, vec!["Checking requirements."]);
        assert_eq!(
            summary.private_reasoning.as_deref(),
            Some("private raw chain")
        );
        assert!(
            !summary
                .reasoning_summaries
                .join("")
                .contains("private raw chain")
        );
    }

    #[test]
    fn repairs_control_characters_and_windows_paths_in_tool_arguments() {
        let value = parse_tool_arguments(
            "{\u{0}\"path\":\"sorting-visualizer\\style.css\",\"content\":\"line one\nline two\tend\"}",
        )
        .expect("repairable tool arguments");
        assert_eq!(value["path"], "sorting-visualizer\\style.css");
        assert_eq!(value["content"], "line one\nline two\tend");
    }

    #[test]
    fn normalizes_anthropic_named_events_and_partial_json() {
        let mut normalizer = ProtocolNormalizer::new(ProviderProtocol::AnthropicMessages);
        for frame in [
            frame(
                Some("message_start"),
                serde_json::json!({"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":8}}}),
            ),
            frame(
                Some("content_block_start"),
                serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_1","name":"patch"}}),
            ),
            frame(
                Some("content_block_delta"),
                serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}),
            ),
            frame(
                Some("message_delta"),
                serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
            ),
            frame(
                Some("message_stop"),
                serde_json::json!({"type":"message_stop"}),
            ),
        ] {
            normalizer.ingest(frame).unwrap();
        }
        let summary = normalizer.finish().unwrap();
        assert_eq!(summary.response_id, "msg_1");
        assert_eq!(summary.tool_calls[0].arguments, serde_json::json!({"x":1}));
        assert_eq!(summary.usage.input_tokens, Some(8));
        assert_eq!(summary.usage.output_tokens, Some(3));
    }
}
