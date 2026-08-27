use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    inference::{
        capabilities::ProviderCapabilities,
        provider::{InferenceProvider, ProviderCredential, ToolRunner},
        ContentPart, FinishReason, InferenceDelta, InferenceRequest, Role, TokenUsage, ToolCall,
        ToolCallDelta,
    },
    streaming::{DeltaSink, StreamError},
};

const CHAT_COMPLETIONS_URL: &str = "https://api.together.ai/v1/chat/completions";

pub(crate) struct TogetherProvider {
    client: Client,
}

impl TogetherProvider {
    pub(crate) fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait]
impl InferenceProvider for TogetherProvider {
    fn id(&self) -> &'static str {
        "together"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            reasoning: true,
            ..ProviderCapabilities::TEXT_STREAMING
        }
    }

    async fn stream(
        &self,
        request: &InferenceRequest,
        credential: &ProviderCredential,
        _tools: Option<&dyn ToolRunner>,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError> {
        let api_key = credential.api_key().ok_or_else(|| {
            StreamError::new("Together requires an API key for the selected model.")
        })?;
        let payload = TogetherRequest::from_canonical(request)?;
        let response = self
            .client
            .post(CHAT_COMPLETIONS_URL)
            .header(header::AUTHORIZATION, format!("Bearer {api_key}"))
            .header(header::ACCEPT, "text/event-stream")
            .json(&payload)
            .send()
            .await
            .map_err(|error| StreamError::new(format!("Could not reach Together: {error}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(together_http_error(status, &body));
        }

        let mut decoder = SseDecoder::default();
        let mut assembler = ToolCallAssembler::default();
        let mut bytes = response.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| {
                StreamError::new(format!("Together's stream ended early: {error}"))
            })?;
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    assembler.flush(sink)?;
                    return Ok(());
                }
                emit_chunk(&data, sink, &mut assembler)?;
            }
        }

        for data in decoder.finish()? {
            if data != "[DONE]" {
                emit_chunk(&data, sink, &mut assembler)?;
            }
        }
        assembler.flush(sink)?;
        Ok(())
    }
}

#[derive(Serialize)]
struct TogetherRequest<'a> {
    model: &'a str,
    messages: Vec<TogetherMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<TogetherToolDeclaration<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

impl<'a> TogetherRequest<'a> {
    fn from_canonical(request: &'a InferenceRequest) -> Result<Self, StreamError> {
        if request.messages.is_empty() {
            return Err(StreamError::new("Together requires at least one message."));
        }
        let messages = request
            .messages
            .iter()
            .map(|message| {
                let role = match message.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                // Text-only messages keep the plain-string wire shape they
                // have always had; an image anywhere switches the message to
                // the OpenAI vision parts array.
                let has_images = message
                    .content
                    .iter()
                    .any(|part| matches!(part, ContentPart::Image { .. }));
                let content = if has_images {
                    TogetherContent::Parts(
                        message
                            .content
                            .iter()
                            .map(|part| match part {
                                ContentPart::Text { text } => TogetherContentPart::Text {
                                    text: text.clone(),
                                },
                                ContentPart::Image { media_type, data } => {
                                    TogetherContentPart::ImageUrl {
                                        image_url: TogetherImageUrl {
                                            url: format!("data:{media_type};base64,{data}"),
                                        },
                                    }
                                }
                            })
                            .collect(),
                    )
                } else {
                    TogetherContent::Text(
                        message
                            .content
                            .iter()
                            .map(|part| match part {
                                ContentPart::Text { text } => text.as_str(),
                                ContentPart::Image { .. } => unreachable!(),
                            })
                            .collect::<String>(),
                    )
                };
                TogetherMessage {
                    role,
                    content,
                    tool_calls: (!message.tool_calls.is_empty()).then(|| {
                        message
                            .tool_calls
                            .iter()
                            .map(|call| TogetherToolCallOut {
                                id: &call.id,
                                kind: "function",
                                function: TogetherFunctionOut {
                                    name: &call.name,
                                    arguments: &call.arguments,
                                },
                            })
                            .collect()
                    }),
                    tool_call_id: message.tool_call_id.as_deref(),
                }
            })
            .collect();

        let tools = (!request.tools.is_empty()).then(|| {
            request
                .tools
                .iter()
                .map(|tool| TogetherToolDeclaration {
                    kind: "function",
                    function: TogetherFunctionDeclaration {
                        name: &tool.name,
                        description: &tool.description,
                        parameters: &tool.parameters,
                    },
                })
                .collect()
        });

        Ok(Self {
            model: &request.target.model_id,
            messages,
            stream: true,
            tool_choice: tools.is_some().then_some("auto"),
            tools,
        })
    }
}

#[derive(Serialize)]
struct TogetherMessage<'a> {
    role: &'static str,
    content: TogetherContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<TogetherToolCallOut<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
}

/// OpenAI-compatible message content: a bare string for text, an array of
/// typed parts the moment images ride along.
#[derive(Serialize)]
#[serde(untagged)]
enum TogetherContent {
    Text(String),
    Parts(Vec<TogetherContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TogetherContentPart {
    Text { text: String },
    ImageUrl { image_url: TogetherImageUrl },
}

#[derive(Serialize)]
struct TogetherImageUrl {
    url: String,
}

#[derive(Serialize)]
struct TogetherToolDeclaration<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: TogetherFunctionDeclaration<'a>,
}

#[derive(Serialize)]
struct TogetherFunctionDeclaration<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
struct TogetherToolCallOut<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: TogetherFunctionOut<'a>,
}

#[derive(Serialize)]
struct TogetherFunctionOut<'a> {
    name: &'a str,
    arguments: &'a str,
}

#[derive(Deserialize)]
struct TogetherChunk {
    #[serde(default)]
    choices: Vec<TogetherChoice>,
    usage: Option<TogetherUsage>,
    error: Option<TogetherStreamError>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TogetherStreamError {
    Message(String),
    Detail { message: String },
}

impl TogetherStreamError {
    fn message(self) -> String {
        match self {
            Self::Message(message) | Self::Detail { message } => message,
        }
    }
}

#[derive(Deserialize)]
struct TogetherChoice {
    delta: TogetherDelta,
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct TogetherDelta {
    content: Option<String>,
    #[serde(alias = "reasoning_content")]
    reasoning: Option<String>,
    tool_calls: Option<Vec<TogetherToolCallDelta>>,
}

/// One streamed tool-call fragment. The first fragment for an `index`
/// carries the id + name; later ones append `arguments` text.
#[derive(Deserialize)]
struct TogetherToolCallDelta {
    #[serde(default)]
    index: usize,
    id: Option<String>,
    function: Option<TogetherFunctionDelta>,
}

#[derive(Deserialize)]
struct TogetherFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

/// Accumulates streamed tool-call fragments; `flush` emits each completed
/// call exactly once, ahead of the Finish delta the same chunk carries.
struct AssembledToolCall {
    call: ToolCall,
    emitted_arguments: usize,
}

#[derive(Default)]
struct ToolCallAssembler {
    calls: Vec<AssembledToolCall>,
    flushed: bool,
}

impl ToolCallAssembler {
    fn absorb(
        &mut self,
        fragment: TogetherToolCallDelta,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError> {
        while self.calls.len() <= fragment.index {
            self.calls.push(AssembledToolCall {
                call: ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                },
                emitted_arguments: 0,
            });
        }
        let assembled = &mut self.calls[fragment.index];
        let call = &mut assembled.call;
        if let Some(id) = fragment.id.filter(|id| !id.is_empty()) {
            call.id = id;
        }
        if let Some(function) = fragment.function {
            if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                call.name = name;
            }
            if let Some(arguments) = function.arguments {
                call.arguments.push_str(&arguments);
            }
        }
        // The provider boundary guarantees identity before a fragment becomes
        // canonical. If arguments arrived unusually early, they stay buffered
        // and are emitted once a later fragment supplies id + name.
        if !call.id.is_empty()
            && !call.name.is_empty()
            && call.arguments.len() > assembled.emitted_arguments
        {
            let arguments_delta = call.arguments[assembled.emitted_arguments..].to_owned();
            assembled.emitted_arguments = call.arguments.len();
            sink.emit_delta(InferenceDelta::ToolCallDelta(ToolCallDelta {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments_delta,
            }))?;
        }
        Ok(())
    }

    fn flush(&mut self, sink: &dyn DeltaSink<InferenceDelta>) -> Result<(), StreamError> {
        if self.flushed {
            return Ok(());
        }
        self.flushed = true;
        for assembled in self.calls.drain(..) {
            if assembled.call.name.is_empty() {
                continue;
            }
            sink.emit_delta(InferenceDelta::ToolCall(assembled.call))?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct TogetherUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

fn emit_chunk(
    data: &str,
    sink: &dyn DeltaSink<InferenceDelta>,
    assembler: &mut ToolCallAssembler,
) -> Result<(), StreamError> {
    let chunk: TogetherChunk = serde_json::from_str(data).map_err(|error| {
        StreamError::new(format!("Together sent an invalid stream event: {error}"))
    })?;
    if let Some(error) = chunk.error {
        return Err(StreamError::new(format!(
            "Together's stream failed: {}",
            error.message()
        )));
    }
    for choice in chunk.choices {
        if let Some(text) = choice.delta.content.filter(|text| !text.is_empty()) {
            sink.emit_delta(InferenceDelta::Text { text })?;
        }
        if let Some(text) = choice.delta.reasoning.filter(|text| !text.is_empty()) {
            sink.emit_delta(InferenceDelta::Reasoning { text })?;
        }
        for fragment in choice.delta.tool_calls.into_iter().flatten() {
            assembler.absorb(fragment, sink)?;
        }
        if let Some(reason) = choice.finish_reason {
            // Assembled calls must land before Finish — the chat loop reads
            // them off the sink to decide whether another round is owed.
            assembler.flush(sink)?;
            sink.emit_delta(InferenceDelta::Finish(FinishReason::from_provider(&reason)))?;
        }
    }
    if let Some(usage) = chunk.usage {
        sink.emit_delta(InferenceDelta::Usage(TokenUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }))?;
    }
    Ok(())
}

fn together_http_error(status: StatusCode, body: &str) -> StreamError {
    #[derive(Deserialize)]
    struct ErrorEnvelope {
        error: Option<ErrorDetail>,
        message: Option<String>,
    }
    #[derive(Deserialize)]
    struct ErrorDetail {
        message: Option<String>,
    }

    let detail = serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .and_then(|error| {
            error
                .error
                .and_then(|detail| detail.message)
                .or(error.message)
        });
    let category = match status.as_u16() {
        401 | 403 => "Together rejected the API key",
        402 => "Together reports insufficient credits",
        404 => "Together could not find the selected model",
        429 => "Together's rate limit was reached",
        500 | 503 | 504 => "Together is temporarily unavailable",
        _ => "Together rejected the request",
    };
    StreamError::new(match detail {
        Some(detail) if !detail.trim().is_empty() => format!("{category}: {detail}"),
        _ => format!("{category} (HTTP {}).", status.as_u16()),
    })
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, StreamError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((end, delimiter_length)) = event_boundary(&self.buffer) {
            let frame = self.buffer.drain(..end).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_length);
            if let Some(data) = decode_frame(&frame)? {
                events.push(data);
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<String>, StreamError> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }
        let frame = std::mem::take(&mut self.buffer);
        Ok(decode_frame(&frame)?.into_iter().collect())
    }
}

fn event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let windows_boundary = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    let unix_boundary = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    match (windows_boundary, unix_boundary) {
        (Some(windows), Some(unix)) => Some(if windows.0 < unix.0 { windows } else { unix }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn decode_frame(frame: &[u8]) -> Result<Option<String>, StreamError> {
    let frame = std::str::from_utf8(frame)
        .map_err(|_| StreamError::new("Together sent non-UTF-8 stream data."))?;
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((!data.is_empty()).then_some(data))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{emit_chunk, together_http_error, SseDecoder, TogetherRequest};
    use crate::{
        inference::{
            FinishReason, InferenceDelta, InferenceMessage, InferenceRequest, ModelTarget, Role,
            ToolCall,
        },
        streaming::{DeltaSink, StreamError},
    };

    #[derive(Default)]
    struct Collector(Mutex<Vec<InferenceDelta>>);

    impl DeltaSink<InferenceDelta> for Collector {
        fn emit_delta(&self, delta: InferenceDelta) -> Result<(), StreamError> {
            self.0.lock().expect("collector should lock").push(delta);
            Ok(())
        }
    }

    #[test]
    fn canonical_messages_map_to_together_without_provider_types_leaking_out() {
        let request = InferenceRequest {
            id: "request-1".to_owned(),
            target: ModelTarget {
                provider_id: "together".to_owned(),
                model_id: "meta-llama/test".to_owned(),
            },
            messages: vec![InferenceMessage::text(Role::System, "Be concise.")],
            tools: Vec::new(),
            session_id: None,
        };
        let mapped = TogetherRequest::from_canonical(&request).expect("request should map");
        let value = serde_json::to_value(mapped).expect("request should serialize");
        assert_eq!(value["model"], "meta-llama/test");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][0]["content"], "Be concise.");
        assert_eq!(value["stream"], true);
        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
    }

    #[test]
    fn image_messages_map_to_the_vision_parts_array_and_text_stays_a_string() {
        let request = InferenceRequest {
            id: "request-3".to_owned(),
            target: ModelTarget {
                provider_id: "together".to_owned(),
                model_id: "meta-llama/test".to_owned(),
            },
            messages: vec![
                InferenceMessage::text(Role::System, "Be concise."),
                InferenceMessage::user_with_images(
                    "What is in this picture?",
                    vec![("image/png".to_owned(), "aGk=".to_owned())],
                ),
            ],
            tools: Vec::new(),
            session_id: None,
        };
        let mapped = TogetherRequest::from_canonical(&request).expect("request should map");
        let value = serde_json::to_value(mapped).expect("request should serialize");
        // Text-only stays the plain string it has always been…
        assert_eq!(value["messages"][0]["content"], "Be concise.");
        // …and the image turn becomes the typed parts array, image first.
        assert_eq!(value["messages"][1]["content"][0]["type"], "image_url");
        assert_eq!(
            value["messages"][1]["content"][0]["image_url"]["url"],
            "data:image/png;base64,aGk="
        );
        assert_eq!(value["messages"][1]["content"][1]["type"], "text");
        assert_eq!(value["messages"][1]["content"][1]["text"], "What is in this picture?");
    }

    #[test]
    fn tool_declarations_and_results_map_to_the_openai_wire_shape() {
        let request = InferenceRequest {
            id: "request-2".to_owned(),
            target: ModelTarget {
                provider_id: "together".to_owned(),
                model_id: "meta-llama/test".to_owned(),
            },
            messages: vec![
                InferenceMessage::text(Role::User, "What do you remember?"),
                InferenceMessage::assistant_tool_calls(
                    String::new(),
                    vec![ToolCall {
                        id: "call-1".to_owned(),
                        name: "recall_memory".to_owned(),
                        arguments: r#"{"name":"the-memory"}"#.to_owned(),
                    }],
                ),
                InferenceMessage::tool_result("call-1".to_owned(), "the full body"),
            ],
            tools: vec![crate::inference::ToolDeclaration {
                name: "recall_memory".to_owned(),
                description: "Fetch one memory.".to_owned(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            session_id: None,
        };
        let mapped = TogetherRequest::from_canonical(&request).expect("request should map");
        let value = serde_json::to_value(mapped).expect("request should serialize");
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["function"]["name"], "recall_memory");
        assert_eq!(value["messages"][1]["tool_calls"][0]["id"], "call-1");
        assert_eq!(
            value["messages"][1]["tool_calls"][0]["function"]["arguments"],
            r#"{"name":"the-memory"}"#
        );
        assert_eq!(value["messages"][2]["role"], "tool");
        assert_eq!(value["messages"][2]["tool_call_id"], "call-1");
        assert_eq!(value["messages"][2]["content"], "the full body");
    }

    #[test]
    fn streamed_tool_call_fragments_assemble_and_flush_before_finish() {
        let collector = Collector::default();
        let mut assembler = super::ToolCallAssembler::default();
        emit_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-9","function":{"name":"recall_memory","arguments":"{\"na"}}]},"finish_reason":null}]}"#,
            &collector,
            &mut assembler,
        )
        .expect("first fragment should absorb");
        emit_chunk(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"me\":\"x\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            &collector,
            &mut assembler,
        )
        .expect("final fragment should flush");

        let events = collector.0.lock().expect("collector should lock");
        assert_eq!(
            events[0],
            InferenceDelta::ToolCallDelta(crate::inference::ToolCallDelta {
                id: "call-9".to_owned(),
                name: "recall_memory".to_owned(),
                arguments_delta: r#"{"na"#.to_owned(),
            })
        );
        assert_eq!(
            events[1],
            InferenceDelta::ToolCallDelta(crate::inference::ToolCallDelta {
                id: "call-9".to_owned(),
                name: "recall_memory".to_owned(),
                arguments_delta: r#"me":"x"}"#.to_owned(),
            })
        );
        assert_eq!(
            events[2],
            InferenceDelta::ToolCall(ToolCall {
                id: "call-9".to_owned(),
                name: "recall_memory".to_owned(),
                arguments: r#"{"name":"x"}"#.to_owned(),
            })
        );
        assert_eq!(
            events[3],
            InferenceDelta::Finish(FinishReason::ToolCalls)
        );
    }

    #[test]
    fn decoder_handles_network_boundaries_and_done_marker() {
        let mut decoder = SseDecoder::default();
        assert!(decoder
            .push(b"data: {\"choices\":[{\"del")
            .unwrap()
            .is_empty());
        let events = decoder
            .push(b"ta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")
            .expect("events should decode");
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("\"content\":\"Hi\""));
        assert_eq!(events[1], "[DONE]");
    }

    #[test]
    fn chunk_normalizes_text_finish_and_usage() {
        let collector = Collector::default();
        emit_chunk(
            r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#,
            &collector,
            &mut super::ToolCallAssembler::default(),
        )
        .expect("chunk should normalize");
        let events = collector.0.lock().expect("collector should lock");
        assert_eq!(
            events[0],
            InferenceDelta::Text {
                text: "Hello".to_owned()
            }
        );
        assert_eq!(events[1], InferenceDelta::Finish(FinishReason::Stop));
        assert!(matches!(events[2], InferenceDelta::Usage(_)));
    }

    #[test]
    fn together_errors_are_safe_and_actionable() {
        let error = together_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"invalid token"}}"#,
        );
        assert_eq!(
            error.to_string(),
            "Together rejected the API key: invalid token"
        );
    }
}
