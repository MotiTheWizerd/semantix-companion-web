mod test_source;

use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;

pub(crate) use test_source::TestTextSource;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextStreamRequest {
    pub(crate) input: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TextStreamEvent {
    Started,
    Delta { sequence: u64, text: String },
    Completed,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StreamError {
    message: String,
}

impl StreamError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for StreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StreamError {}

pub(crate) trait TextDeltaSink: Send + Sync {
    fn emit_delta(&self, text: String) -> Result<(), StreamError>;
}

pub(crate) trait TextStreamSink: Send + Sync {
    fn emit(&self, event: TextStreamEvent) -> Result<(), StreamError>;
}

#[async_trait]
pub(crate) trait TextStreamSource: Send + Sync {
    async fn stream(
        &self,
        request: &TextStreamRequest,
        sink: &dyn TextDeltaSink,
    ) -> Result<(), StreamError>;
}

pub(crate) struct StreamingService {
    source: Arc<dyn TextStreamSource>,
}

impl StreamingService {
    pub(crate) fn new(source: Arc<dyn TextStreamSource>) -> Self {
        Self { source }
    }

    pub(crate) async fn stream(
        &self,
        request: &TextStreamRequest,
        sink: &dyn TextStreamSink,
    ) -> Result<(), StreamError> {
        sink.emit(TextStreamEvent::Started)?;
        let sequenced_sink = SequencedDeltaSink::new(sink);

        match self.source.stream(request, &sequenced_sink).await {
            Ok(()) => sink.emit(TextStreamEvent::Completed),
            Err(error) => {
                let _ = sink.emit(TextStreamEvent::Failed {
                    message: error.to_string(),
                });
                Err(error)
            }
        }
    }
}

struct SequencedDeltaSink<'a> {
    sink: &'a dyn TextStreamSink,
    next_sequence: std::sync::atomic::AtomicU64,
}

impl<'a> SequencedDeltaSink<'a> {
    fn new(sink: &'a dyn TextStreamSink) -> Self {
        Self {
            sink,
            next_sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl TextDeltaSink for SequencedDeltaSink<'_> {
    fn emit_delta(&self, text: String) -> Result<(), StreamError> {
        let sequence = self
            .next_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.sink.emit(TextStreamEvent::Delta { sequence, text })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{
        StreamError, StreamingService, TextDeltaSink, TextStreamEvent, TextStreamRequest,
        TextStreamSink, TextStreamSource,
    };

    struct TwoChunkSource;

    #[async_trait]
    impl TextStreamSource for TwoChunkSource {
        async fn stream(
            &self,
            _request: &TextStreamRequest,
            sink: &dyn TextDeltaSink,
        ) -> Result<(), StreamError> {
            sink.emit_delta("one ".to_owned())?;
            sink.emit_delta("two".to_owned())
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<TextStreamEvent>>,
    }

    impl TextStreamSink for RecordingSink {
        fn emit(&self, event: TextStreamEvent) -> Result<(), StreamError> {
            self.events
                .lock()
                .expect("recording sink should not be poisoned")
                .push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn streaming_service_owns_ordered_lifecycle_events() {
        let service = StreamingService::new(std::sync::Arc::new(TwoChunkSource));
        let sink = RecordingSink::default();

        service
            .stream(
                &TextStreamRequest {
                    input: "test".to_owned(),
                },
                &sink,
            )
            .await
            .expect("stream should complete");

        assert_eq!(
            *sink.events.lock().expect("events should be readable"),
            vec![
                TextStreamEvent::Started,
                TextStreamEvent::Delta {
                    sequence: 0,
                    text: "one ".to_owned(),
                },
                TextStreamEvent::Delta {
                    sequence: 1,
                    text: "two".to_owned(),
                },
                TextStreamEvent::Completed,
            ]
        );
    }
}
