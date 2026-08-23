use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use super::{
    capabilities::ProviderCapabilities,
    provider::{InferenceProvider, ProviderCredential, ToolRunner},
    providers::{ClaudeProvider, TestProvider, TogetherProvider},
    InferenceDelta, InferenceRequest,
};
use crate::streaming::{DeltaSink, StreamError, StreamSource};

pub(crate) struct InferenceExecution {
    pub(crate) request: InferenceRequest,
    pub(crate) credential: ProviderCredential,
    /// Set for providers that run tools inside their own turn. Belongs to the
    /// execution, not the request: it is machinery for THIS run, not data.
    pub(crate) tool_runner: Option<Arc<dyn ToolRunner>>,
}

pub(crate) struct InferenceGateway {
    providers: HashMap<&'static str, Arc<dyn InferenceProvider>>,
}

impl Default for InferenceGateway {
    fn default() -> Self {
        Self::new([
            Arc::new(TestProvider::default()) as Arc<dyn InferenceProvider>,
            Arc::new(TogetherProvider::new()),
            Arc::new(ClaudeProvider::default()),
        ])
    }
}

impl InferenceGateway {
    fn new<const N: usize>(providers: [Arc<dyn InferenceProvider>; N]) -> Self {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.id(), provider))
                .collect(),
        }
    }
}

#[async_trait]
impl StreamSource for InferenceGateway {
    type Request = InferenceExecution;
    type Delta = InferenceDelta;

    async fn stream(
        &self,
        execution: &Self::Request,
        sink: &dyn DeltaSink<Self::Delta>,
    ) -> Result<(), StreamError> {
        let provider_id = execution.request.target.provider_id.as_str();
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            StreamError::new(format!(
                "Provider '{provider_id}' is not connected to Companion yet."
            ))
        })?;
        let requested = ProviderCapabilities::TEXT_STREAMING;
        if !provider.capabilities().supports(&requested) {
            return Err(StreamError::new(format!(
                "Provider '{provider_id}' cannot satisfy this streaming request."
            )));
        }

        provider
            .stream(
                &execution.request,
                &execution.credential,
                execution.tool_runner.as_deref(),
                sink,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::InferenceGateway;

    #[test]
    fn gateway_registers_the_test_together_and_claude_providers() {
        let gateway = InferenceGateway::default();
        assert!(gateway.providers.contains_key("test"));
        assert!(gateway.providers.contains_key("together"));
        assert!(gateway.providers.contains_key("claude_code"));
    }
}
