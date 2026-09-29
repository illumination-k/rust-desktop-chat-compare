use crate::provider;
use crate::stream::{self, StreamHandle};
use crate::{AppPaths, ConversationStore, Result, Settings, SettingsStore, Workspace};

/// Everything a UI needs: the workspace, settings, and a way to start a turn.
#[derive(Debug)]
pub struct ChatService {
    pub workspace: Workspace,
    settings: Settings,
    settings_store: SettingsStore,
}

impl ChatService {
    /// Loads from the OS-standard locations (or `CHAT_COMPARE_HOME`).
    pub fn load() -> Result<Self> {
        Self::load_from(&AppPaths::from_env()?)
    }

    pub fn load_from(paths: &AppPaths) -> Result<Self> {
        let settings_store = SettingsStore::new(&paths.settings_file);
        Ok(Self {
            workspace: Workspace::load(ConversationStore::new(&paths.conversations_dir))?,
            settings: settings_store.load()?,
            settings_store,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn save_settings(&mut self, settings: Settings) -> Result<()> {
        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(())
    }

    /// Records the user message and starts streaming the reply on the current tokio runtime.
    /// Returns the conversation id the stream belongs to.
    ///
    /// # Panics
    ///
    /// Panics when called outside a tokio runtime context.
    pub fn send(&mut self, text: &str) -> Result<(String, StreamHandle)> {
        // Resolve the provider first so a missing API key does not leave a dangling turn.
        let provider = provider::from_settings(&self.settings)?;
        let (id, request) = self.workspace.begin_turn(text, &self.settings)?;
        Ok((id, stream::start(provider, request)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderKind, StreamEvent, StreamOutcome};

    #[tokio::test]
    async fn send_streams_mock_reply_into_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(dir.path().to_owned());
        let mut service = ChatService::load_from(&paths).unwrap();
        service
            .save_settings(Settings {
                provider: ProviderKind::Mock,
                ..Settings::default()
            })
            .unwrap();

        let (id, mut handle) = service.send("hi").unwrap();
        handle.cancel();
        while let Some(event) = handle.events.recv().await {
            match event {
                StreamEvent::Delta(text) => service.workspace.append_delta(&id, &text),
                StreamEvent::ToolUse(call) => service.workspace.record_tool_use(&id, &call),
                StreamEvent::Finished(outcome) => {
                    service.workspace.finish_turn(&id, &outcome).unwrap();
                    assert_eq!(outcome, StreamOutcome::Cancelled);
                }
            }
        }
        let reloaded = ChatService::load_from(&paths).unwrap();
        assert_eq!(reloaded.settings().provider, ProviderKind::Mock);
        let messages = &reloaded.workspace.active().unwrap().messages;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].error.as_deref(), Some("Cancelled"));
    }
}
