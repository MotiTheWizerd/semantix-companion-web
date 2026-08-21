use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    inference::{
        capabilities::ProviderCapabilities,
        provider::{InferenceProvider, ProviderCredential},
        ContentPart, FinishReason, InferenceDelta, InferenceRequest, Role, TokenUsage,
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
        let mut bytes = response.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| {
                StreamError::new(format!("Together's stream ended early: {error}"))
            })?;
            for data in decoder.push(&chunk)? {
                if data == "[DONE]" {
                    return Ok(());
                }
                emit_chunk(&data, sink)?;
            }
        }

        for data in decoder.finish()? {
            if data != "[DONE]" {
                emit_chunk(&data, sink)?;
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct TogetherRequest<'a> {
    model: &'a str,
    messages: Vec<TogetherMessage>,
    stream: bool,
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
                };
                let content = message
                    .content
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => text.as_str(),
                    })
                    .collect::<String>();
                TogetherMessage { role, content }
            })
            .collect();

        Ok(Self {
            model: &request.target.model_id,
            messages,
            stream: true,
        })
    }
}

#[derive(Serialize)]
struct TogetherMessage {
    role: &'static str,
    content: String,
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
}

#[derive(Deserialize)]
struct TogetherUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

fn emit_chunk(data: &str, sink: &dyn DeltaSink<InferenceDelta>) -> Result<(), StreamError> {
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
        if let Some(reason) = choice.finish_reason {
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
            ContentPart, FinishReason, InferenceDelta, InferenceMessage, InferenceRequest,
            ModelTarget, Role,
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
            messages: vec![InferenceMessage {
                role: Role::System,
                content: vec![ContentPart::Text {
                    text: "Be concise.".to_owned(),
                }],
            }],
        };
        let mapped = TogetherRequest::from_canonical(&request).expect("request should map");
        let value = serde_json::to_value(mapped).expect("request should serialize");
        assert_eq!(value["model"], "meta-llama/test");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][0]["content"], "Be concise.");
        assert_eq!(value["stream"], true);
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
