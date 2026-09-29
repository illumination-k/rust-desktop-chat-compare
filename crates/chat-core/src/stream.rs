//! Runs a provider in the background and exposes its output through a UI-agnostic channel.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::DeltaSink;
use crate::{ChatRequest, Provider};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// A chunk of assistant text.
    Delta(String),
    /// Always the last event of a stream.
    Finished(StreamOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamOutcome {
    Completed,
    Cancelled,
    Failed(String),
}

/// Receiver side of a running stream. Dropping it does not stop the stream; call [`cancel`](Self::cancel).
#[derive(Debug)]
pub struct StreamHandle {
    pub events: mpsc::UnboundedReceiver<StreamEvent>,
    pub canceller: Canceller,
}

impl StreamHandle {
    pub fn cancel(&self) {
        self.canceller.cancel();
    }
}

/// Cloneable handle to cancel a running stream.
#[derive(Clone, Debug, Default)]
pub struct Canceller(CancellationToken);

impl Canceller {
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

/// Spawns `provider` on the current tokio runtime.
///
/// # Panics
///
/// Panics when called outside a tokio runtime context.
pub fn start(provider: Arc<dyn Provider>, request: ChatRequest) -> StreamHandle {
    let (tx, events) = mpsc::unbounded_channel();
    let canceller = Canceller::default();
    let token = canceller.0.clone();
    tokio::spawn(async move {
        let outcome = tokio::select! {
            () = token.cancelled() => StreamOutcome::Cancelled,
            result = provider.stream(request, DeltaSink(tx.clone())) => match result {
                Ok(()) => StreamOutcome::Completed,
                Err(e) => StreamOutcome::Failed(e.to_string()),
            },
        };
        let _ = tx.send(StreamEvent::Finished(outcome));
    });
    StreamHandle { events, canceller }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::provider::MockProvider;

    fn request() -> ChatRequest {
        ChatRequest {
            model: String::new(),
            system_prompt: String::new(),
            max_tokens: 1,
            messages: Vec::new(),
        }
    }

    #[tokio::test]
    async fn mock_stream_completes_with_full_text() {
        let provider = MockProvider {
            response: "a b c".into(),
            delay: Duration::ZERO,
        };
        let mut handle = start(Arc::new(provider), request());
        let mut text = String::new();
        let outcome = loop {
            match handle.events.recv().await.unwrap() {
                StreamEvent::Delta(t) => text.push_str(&t),
                StreamEvent::Finished(o) => break o,
            }
        };
        assert_eq!(text, "a b c");
        assert_eq!(outcome, StreamOutcome::Completed);
    }

    #[tokio::test]
    async fn cancel_stops_the_stream() {
        let provider = MockProvider {
            response: "a b c".into(),
            delay: Duration::from_secs(60),
        };
        let mut handle = start(Arc::new(provider), request());
        handle.cancel();
        assert_eq!(
            handle.events.recv().await,
            Some(StreamEvent::Finished(StreamOutcome::Cancelled))
        );
    }
}
