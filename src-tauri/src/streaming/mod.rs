use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StreamEvent<Delta> {
    Started,
    Delta { sequence: u64, payload: Delta },
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

pub(crate) trait DeltaSink<Delta>: Send + Sync {
    fn emit_delta(&self, payload: Delta) -> Result<(), StreamError>;
}

pub(crate) trait StreamSink<Delta>: Send + Sync {
    fn emit(&self, event: StreamEvent<Delta>) -> Result<(), StreamError>;
}

#[async_trait]
pub(crate) trait StreamSource: Send + Sync {
    type Request: Send + Sync;
    type Delta: Send + Sync;

    async fn stream(
        &self,
        request: &Self::Request,
        sink: &dyn DeltaSink<Self::Delta>,
    ) -> Result<(), StreamError>;
}

pub(crate) struct StreamingService<Request, Delta> {
    source: Arc<dyn StreamSource<Request = Request, Delta = Delta>>,
}

impl<Request, Delta> StreamingService<Request, Delta>
where
    Request: Send + Sync,
    Delta: Send + Sync,
{
    pub(crate) fn new(source: Arc<dyn StreamSource<Request = Request, Delta = Delta>>) -> Self {
        Self { source }
    }

    pub(crate) async fn stream(
        &self,
        request: &Request,
        sink: &dyn StreamSink<Delta>,
    ) -> Result<(), StreamError> {
        sink.emit(StreamEvent::Started)?;
        let sequenced_sink = SequencedDeltaSink::new(sink);

        match self.source.stream(request, &sequenced_sink).await {
            Ok(()) => sink.emit(StreamEvent::Completed),
            Err(error) => {
                let _ = sink.emit(StreamEvent::Failed {
                    message: error.to_string(),
                });
                Err(error)
            }
        }
    }
}

struct SequencedDeltaSink<'a, Delta> {
    sink: &'a dyn StreamSink<Delta>,
    next_sequence: std::sync::atomic::AtomicU64,
}

impl<'a, Delta> SequencedDeltaSink<'a, Delta> {
    fn new(sink: &'a dyn StreamSink<Delta>) -> Self {
        Self {
            sink,
            next_sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl<Delta: Send + Sync> DeltaSink<Delta> for SequencedDeltaSink<'_, Delta> {
    fn emit_delta(&self, payload: Delta) -> Result<(), StreamError> {
        let sequence = self
            .next_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.sink.emit(StreamEvent::Delta { sequence, payload })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{DeltaSink, StreamError, StreamEvent, StreamSink, StreamSource, StreamingService};

    struct TwoChunkSource;

    #[async_trait]
    impl StreamSource for TwoChunkSource {
        type Request = String;
        type Delta = String;

        async fn stream(
            &self,
            _request: &String,
            sink: &dyn DeltaSink<String>,
        ) -> Result<(), StreamError> {
            sink.emit_delta("one ".to_owned())?;
            sink.emit_delta("two".to_owned())
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<StreamEvent<String>>>,
    }

    impl StreamSink<String> for RecordingSink {
        fn emit(&self, event: StreamEvent<String>) -> Result<(), StreamError> {
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
            .stream(&"test".to_owned(), &sink)
            .await
            .expect("stream should complete");

        assert_eq!(
            *sink.events.lock().expect("events should be readable"),
            vec![
                StreamEvent::Started,
                StreamEvent::Delta {
                    sequence: 0,
                    payload: "one ".to_owned(),
                },
                StreamEvent::Delta {
                    sequence: 1,
                    payload: "two".to_owned(),
                },
                StreamEvent::Completed,
            ]
        );
    }
}
