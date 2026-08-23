use async_trait::async_trait;
use zeroize::Zeroizing;

use super::{
    capabilities::ProviderCapabilities,
    protocol::{InferenceDelta, ToolCall},
    InferenceRequest,
};
use crate::streaming::{DeltaSink, StreamError};

/// Runs one tool call and returns what the model should read back.
///
/// Most providers never touch this: they surface a tool call as a delta and
/// the chat loop executes it between rounds. A provider that owns its own
/// agentic loop (Claude Code, which runs the tool inside its turn) needs to
/// execute mid-stream instead, and this is the door it uses — so tool
/// execution stays in ONE place regardless of which lane asked for it.
#[async_trait]
pub(crate) trait ToolRunner: Send + Sync {
    async fn run(&self, call: &ToolCall) -> Result<String, String>;
}

pub(crate) enum ProviderCredential {
    None,
    ApiKey(Zeroizing<String>),
}

impl ProviderCredential {
    pub(crate) fn api_key(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::ApiKey(value) => Some(value.as_str()),
        }
    }
}

#[async_trait]
pub(crate) trait InferenceProvider: Send + Sync {
    fn id(&self) -> &'static str;

    fn capabilities(&self) -> ProviderCapabilities;

    async fn stream(
        &self,
        request: &InferenceRequest,
        credential: &ProviderCredential,
        tools: Option<&dyn ToolRunner>,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError>;
}
