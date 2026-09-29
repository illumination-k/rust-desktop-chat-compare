//! LLM providers. Each provider pushes text deltas into a [`DeltaSink`].

mod anthropic;
mod mock;

use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

pub use anthropic::AnthropicProvider;
pub use mock::MockProvider;

use crate::{ApiKeyStore, ChatRequest, Error, ProviderKind, Result, Settings, StreamEvent};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("unexpected response: {0}")]
    Protocol(String),
}

/// Where a provider writes the text it receives.
#[derive(Clone, Debug)]
pub struct DeltaSink(pub(crate) mpsc::UnboundedSender<StreamEvent>);

impl DeltaSink {
    pub fn send(&self, text: impl Into<String>) {
        // A closed receiver means the UI went away; the stream is cancelled right after.
        let _ = self.0.send(StreamEvent::Delta(text.into()));
    }
}

pub trait Provider: Send + Sync + 'static {
    /// Streams one assistant turn into `sink`, resolving when the response is complete.
    fn stream(
        &self,
        request: ChatRequest,
        sink: DeltaSink,
    ) -> BoxFuture<'_, Result<(), ProviderError>>;
}

/// Builds the provider selected in `settings`, reading the API key from the keychain if needed.
pub fn from_settings(settings: &Settings) -> Result<Arc<dyn Provider>> {
    Ok(match settings.provider {
        ProviderKind::Mock => Arc::new(MockProvider::default()),
        ProviderKind::Anthropic => {
            let key = ApiKeyStore.get()?.ok_or(Error::MissingApiKey)?;
            Arc::new(AnthropicProvider::new(key))
        }
    })
}
