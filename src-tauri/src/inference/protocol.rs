#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelTarget {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InferenceRequest {
    pub(crate) id: String,
    pub(crate) target: ModelTarget,
    pub(crate) messages: Vec<InferenceMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InferenceMessage {
    pub(crate) role: Role,
    pub(crate) content: Vec<ContentPart>,
}

impl InferenceMessage {
    pub(crate) fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContentPart {
    Text { text: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InferenceDelta {
    Text { text: String },
    Reasoning { text: String },
    Usage(TokenUsage),
    Finish(FinishReason),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

impl FinishReason {
    pub(crate) fn from_provider(value: &str) -> Self {
        match value {
            "stop" | "eos" => Self::Stop,
            "length" => Self::Length,
            "tool_calls" | "function_call" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            other => Self::Other(other.to_owned()),
        }
    }
}
