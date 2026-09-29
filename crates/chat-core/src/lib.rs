//! UI-independent logic shared by every desktop app in this comparison.
//!
//! UI crates only glue these pieces to their own event loop:
//!
//! - [`ChatService`] bundles the pieces below; most UIs only talk to it.
//! - [`Workspace`] owns conversations and persists them ([`ConversationStore`]).
//! - [`Settings`] / [`SettingsStore`] / [`ApiKeyStore`] hold user configuration.
//! - [`stream::start`] runs a [`Provider`] on the tokio runtime and exposes the
//!   tokens through a plain `tokio::sync::mpsc` receiver.
//! - [`markdown`] turns assistant output into flat blocks (native UIs) or HTML (web UIs).

mod error;
pub mod markdown;
mod model;
mod paths;
pub mod provider;
mod service;
mod settings;
mod sse;
mod store;
pub mod stream;
mod workspace;

pub use error::{Error, Result};
pub use model::{ChatRequest, Conversation, Message, Role};
pub use paths::AppPaths;
pub use provider::{Provider, ProviderError};
pub use service::ChatService;
pub use settings::{ApiKeyStore, ProviderKind, Settings, SettingsStore};
pub use store::ConversationStore;
pub use stream::{StreamEvent, StreamHandle, StreamOutcome};
pub use workspace::Workspace;
