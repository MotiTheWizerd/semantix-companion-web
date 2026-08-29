mod capabilities;
mod catalog;
mod gateway;
mod protocol;
mod provider;
mod providers;

pub(crate) use catalog::{api_provider_spec, API_PROVIDERS};
pub(crate) use gateway::{InferenceExecution, InferenceGateway};
pub(crate) use protocol::{
    ContentPart, FinishReason, InferenceDelta, InferenceMessage, InferenceRequest, ModelTarget,
    Role, TokenUsage, ToolCall, ToolCallDelta, ToolDeclaration,
};
pub(crate) use provider::{ProviderCredential, ToolRunner};
pub(crate) use providers::set_bundled_sidecar_dir;

#[cfg(test)]
pub(crate) use capabilities::ProviderCapabilities;
#[cfg(test)]
pub(crate) use provider::InferenceProvider;
