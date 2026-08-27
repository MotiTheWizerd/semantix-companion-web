#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelTarget {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InferenceRequest {
    pub(crate) id: String,
    pub(crate) target: ModelTarget,
    pub(crate) messages: Vec<InferenceMessage>,
    /// Tools the model may call this turn. Empty = plain completion; the
    /// provider mapping omits the field entirely so tool-less requests stay
    /// byte-identical to the pre-tooling wire shape.
    pub(crate) tools: Vec<ToolDeclaration>,
    /// Stable identity of the conversation this request continues. Stateless
    /// providers ignore it; a session-keeping provider (Claude Code) uses it
    /// to resume the same underlying session across turns.
    pub(crate) session_id: Option<String>,
}

/// One callable tool, provider-agnostic. `parameters` is a JSON Schema for
/// the arguments object, passed through to the provider untouched.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolDeclaration {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: serde_json::Value,
}

/// A completed tool call as the model emitted it. `arguments` stays the raw
/// JSON string the provider streamed — parsing is the executor's business.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// One provider-normalized fragment of a tool call's JSON arguments.
///
/// Providers own wire assembly: by the time a fragment crosses this seam it
/// has a stable call id and canonical tool name. A provider that cannot expose
/// partial arguments simply omits these deltas and still emits the completed
/// `ToolCall`, so streaming is progressive enhancement rather than a new
/// requirement every future adapter must satisfy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolCallDelta {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments_delta: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InferenceMessage {
    pub(crate) role: Role,
    pub(crate) content: Vec<ContentPart>,
    /// Set on an assistant message that requested tool calls.
    pub(crate) tool_calls: Vec<ToolCall>,
    /// Set on a `Role::Tool` message: which call this result answers.
    pub(crate) tool_call_id: Option<String>,
}

impl InferenceMessage {
    pub(crate) fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A user turn carrying images — the images lead, the words follow, the
    /// order vision models read a "look at this: …" message in.
    pub(crate) fn user_with_images(
        text: impl Into<String>,
        images: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut content: Vec<ContentPart> = images
            .into_iter()
            .map(|(media_type, data)| ContentPart::Image { media_type, data })
            .collect();
        let text = text.into();
        if !text.is_empty() {
            content.push(ContentPart::Text { text });
        }
        Self {
            role: Role::User,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// The assistant turn that asked for tools — text the model produced in
    /// the same round (may be empty) plus the calls themselves.
    pub(crate) fn assistant_tool_calls(text: String, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentPart::Text { text }]
            },
            tool_calls,
            tool_call_id: None,
        }
    }

    /// One executed tool's output, folded back for the model to read.
    pub(crate) fn tool_result(tool_call_id: String, text: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: vec![ContentPart::Text { text: text.into() }],
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContentPart {
    Text { text: String },
    /// One image the model should see. `data` is base64 (no data-URL prefix);
    /// each provider mapping wraps it in its own wire shape.
    Image { media_type: String, data: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InferenceDelta {
    Text { text: String },
    Reasoning { text: String },
    /// A provider-normalized fragment; downstream features may project it into
    /// live UI, but tool execution still waits for the completed `ToolCall`.
    ToolCallDelta(ToolCallDelta),
    /// A fully assembled tool call — providers accumulate the streamed
    /// fragments and emit one of these per call, before the Finish delta.
    ToolCall(ToolCall),
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
