use std::time::Duration;

use async_trait::async_trait;
use tokio::time::sleep;
use uuid::Uuid;

use crate::{
    inference::{
        capabilities::ProviderCapabilities,
        provider::{InferenceProvider, ProviderCredential, ToolRunner},
        FinishReason, InferenceDelta, InferenceRequest,
    },
    streaming::{DeltaSink, StreamError},
};

const TEST_RESPONSES: &[&str] = &[
    "The stream is alive. Each piece of this reply travelled through the reusable event pipeline before reaching the conversation.",
    "Companion heard you. This is a temporary test response, arriving one small chunk at a time.",
    "A clean stream should feel uneventful: ordered pieces, one completed message, and a durable result when you return.",
    "We are not speaking to a model yet. We are proving the path that every future model response will travel.",
    "Signal received. The streaming core is isolated, the chat adapter is listening, and persistence waits at the other end.",
    "One event begins the reply, several ordered deltas build it, and one final event seals it into local history.",
];

pub(crate) struct TestProvider {
    responses: &'static [&'static str],
    chunk_delay: Duration,
}

impl Default for TestProvider {
    fn default() -> Self {
        Self {
            responses: TEST_RESPONSES,
            chunk_delay: Duration::from_millis(55),
        }
    }
}

#[async_trait]
impl InferenceProvider for TestProvider {
    fn id(&self) -> &'static str {
        "test"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::TEXT_STREAMING
    }

    async fn stream(
        &self,
        _request: &InferenceRequest,
        _credential: &ProviderCredential,
        _tools: Option<&dyn ToolRunner>,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError> {
        if self.responses.is_empty() {
            return Err(StreamError::new(
                "the test provider has no configured responses",
            ));
        }

        let random_byte = Uuid::new_v4().as_bytes()[0] as usize;
        let response = self.responses[random_byte % self.responses.len()];
        for chunk in response.split_inclusive(char::is_whitespace) {
            sink.emit_delta(InferenceDelta::Text {
                text: chunk.to_owned(),
            })?;
            if !self.chunk_delay.is_zero() {
                sleep(self.chunk_delay).await;
            }
        }
        sink.emit_delta(InferenceDelta::Finish(FinishReason::Stop))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use super::TestProvider;
    use crate::{
        inference::{
            provider::InferenceProvider, InferenceDelta, InferenceRequest, ModelTarget,
            ProviderCredential,
        },
        streaming::{DeltaSink, StreamError},
    };

    #[derive(Default)]
    struct Collector(Mutex<String>);

    impl DeltaSink<InferenceDelta> for Collector {
        fn emit_delta(&self, delta: InferenceDelta) -> Result<(), StreamError> {
            if let InferenceDelta::Text { text } = delta {
                self.0
                    .lock()
                    .expect("collector should lock")
                    .push_str(&text);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_provider_streams_one_complete_response() {
        const RESPONSES: &[&str] = &["A deterministic streamed response."];
        let provider = TestProvider {
            responses: RESPONSES,
            chunk_delay: Duration::ZERO,
        };
        let collector = Collector::default();
        provider
            .stream(
                &InferenceRequest {
                    id: "request".to_owned(),
                    target: ModelTarget {
                        provider_id: "test".to_owned(),
                        model_id: "test-stream".to_owned(),
                    },
                    messages: Vec::new(),
                    tools: Vec::new(),
                    session_id: None,
                },
                &ProviderCredential::None,
                None,
                &collector,
            )
            .await
            .expect("test provider should stream");

        assert_eq!(
            collector.0.lock().expect("collector should lock").as_str(),
            RESPONSES[0]
        );
    }
}
