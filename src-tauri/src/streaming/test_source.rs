use std::time::Duration;

use async_trait::async_trait;
use tokio::time::sleep;
use uuid::Uuid;

use super::{StreamError, TextDeltaSink, TextStreamRequest, TextStreamSource};

const TEST_RESPONSES: &[&str] = &[
    "The stream is alive. Each piece of this reply travelled through the reusable event pipeline before reaching the conversation.",
    "Companion heard you. This is a temporary test response, arriving one small chunk at a time.",
    "A clean stream should feel uneventful: ordered pieces, one completed message, and a durable result when you return.",
    "We are not speaking to a model yet. We are proving the path that every future model response will travel.",
    "Signal received. The streaming core is isolated, the chat adapter is listening, and persistence waits at the other end.",
    "One event begins the reply, several ordered deltas build it, and one final event seals it into local history.",
];

pub(crate) struct TestTextSource {
    responses: &'static [&'static str],
    chunk_delay: Duration,
}

impl Default for TestTextSource {
    fn default() -> Self {
        Self {
            responses: TEST_RESPONSES,
            chunk_delay: Duration::from_millis(55),
        }
    }
}

#[async_trait]
impl TextStreamSource for TestTextSource {
    async fn stream(
        &self,
        _request: &TextStreamRequest,
        sink: &dyn TextDeltaSink,
    ) -> Result<(), StreamError> {
        if self.responses.is_empty() {
            return Err(StreamError::new(
                "the test stream has no configured responses",
            ));
        }

        let random_byte = Uuid::new_v4().as_bytes()[0] as usize;
        let response = self.responses[random_byte % self.responses.len()];

        for chunk in response.split_inclusive(char::is_whitespace) {
            sink.emit_delta(chunk.to_owned())?;
            if !self.chunk_delay.is_zero() {
                sleep(self.chunk_delay).await;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use super::TestTextSource;
    use crate::streaming::{StreamError, TextDeltaSink, TextStreamRequest, TextStreamSource};

    #[derive(Default)]
    struct DeltaCollector {
        content: Mutex<String>,
    }

    impl TextDeltaSink for DeltaCollector {
        fn emit_delta(&self, text: String) -> Result<(), StreamError> {
            self.content
                .lock()
                .expect("collector should not be poisoned")
                .push_str(&text);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_source_streams_one_complete_configured_response() {
        const RESPONSES: &[&str] = &["A deterministic streamed response."];
        let source = TestTextSource {
            responses: RESPONSES,
            chunk_delay: Duration::ZERO,
        };
        let collector = DeltaCollector::default();

        source
            .stream(
                &TextStreamRequest {
                    input: "hello".to_owned(),
                },
                &collector,
            )
            .await
            .expect("test response should stream");

        assert_eq!(
            collector
                .content
                .lock()
                .expect("content should be readable")
                .as_str(),
            RESPONSES[0]
        );
    }
}
