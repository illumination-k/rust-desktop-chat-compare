use crate::mcp_app::{AppCall, server};
use crate::{
    ChatRequest, Conversation, ConversationStore, Error, Message, Result, Role, Settings,
    StreamOutcome, ToolUse,
};

/// All conversations plus the active selection; the single source of truth for every UI.
///
/// A turn is driven as `begin_turn` → (`append_delta` | `record_tool_use`)* → `finish_turn`.
#[derive(Debug)]
pub struct Workspace {
    store: ConversationStore,
    /// Newest first.
    conversations: Vec<Conversation>,
    active: Option<String>,
}

impl Workspace {
    pub fn load(store: ConversationStore) -> Result<Self> {
        let conversations = store.load_all()?;
        let active = conversations.first().map(|c| c.id.clone());
        Ok(Self {
            store,
            conversations,
            active,
        })
    }

    pub fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&Conversation> {
        self.active.as_deref().and_then(|id| self.get(id))
    }

    pub fn get(&self, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == id)
    }

    pub fn select(&mut self, id: &str) {
        if self.get(id).is_some() {
            self.active = Some(id.to_owned());
        }
    }

    /// Starts a fresh, empty conversation (reusing the current one if it is still empty).
    pub fn new_conversation(&mut self) -> &str {
        if let Some(pos) = self
            .conversations
            .iter()
            .position(|c| c.messages.is_empty())
        {
            let id = self.conversations[pos].id.clone();
            self.active = Some(id);
        } else {
            let conversation = Conversation::new();
            self.active = Some(conversation.id.clone());
            self.conversations.insert(0, conversation);
        }
        self.active.as_deref().unwrap_or_default()
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.store.delete(id)?;
        self.conversations.retain(|c| c.id != id);
        if self.active.as_deref() == Some(id) {
            self.active = self.conversations.first().map(|c| c.id.clone());
        }
        Ok(())
    }

    /// Appends the user message and an empty assistant placeholder to the active
    /// conversation (creating one if needed), then returns its id and the request to stream.
    pub fn begin_turn(&mut self, text: &str, settings: &Settings) -> Result<(String, ChatRequest)> {
        if self.active().is_none() {
            self.new_conversation();
        }
        let id = self.active.clone().unwrap_or_default();
        let conversation = self.get_mut(&id)?;
        conversation.messages.push(Message::user(text));
        conversation.ensure_title();
        let request = ChatRequest {
            model: settings.model.clone(),
            system_prompt: settings.system_prompt.clone(),
            max_tokens: settings.max_tokens,
            messages: conversation
                .messages
                .iter()
                .filter(|m| m.error.is_none() && !m.content.is_empty())
                .cloned()
                .collect(),
        };
        conversation.messages.push(Message::assistant(""));
        conversation.touch();
        self.move_to_top(&id);
        self.save(&id)?;
        Ok((id, request))
    }

    pub fn append_delta(&mut self, id: &str, delta: &str) {
        if let Some(message) = self.get_mut(id).ok().and_then(|c| c.messages.last_mut())
            && message.role == Role::Assistant
        {
            message.content.push_str(delta);
        }
    }

    /// Runs the tool on the (mock) MCP server and attaches the call to the assistant message,
    /// so the UI can render the tool's MCP App view.
    pub fn record_tool_use(&mut self, id: &str, tool_use: &ToolUse) {
        if let Some(message) = self.get_mut(id).ok().and_then(|c| c.messages.last_mut())
            && message.role == Role::Assistant
        {
            match server::call_tool(&tool_use.name, &tool_use.input) {
                Ok(result) => {
                    message.app = Some(AppCall {
                        tool: tool_use.name.clone(),
                        input: tool_use.input.clone(),
                        result,
                    });
                }
                Err(e) => message.error = Some(e),
            }
        }
    }

    pub fn finish_turn(&mut self, id: &str, outcome: &StreamOutcome) -> Result<()> {
        let conversation = self.get_mut(id)?;
        if let Some(message) = conversation.messages.last_mut()
            && message.role == Role::Assistant
        {
            match outcome {
                StreamOutcome::Completed => {}
                StreamOutcome::Cancelled => message.error = Some("Cancelled".to_owned()),
                StreamOutcome::Failed(e) => message.error = Some(e.clone()),
            }
        }
        conversation.touch();
        self.save(id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut Conversation> {
        self.conversations
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::ConversationNotFound(id.to_owned()))
    }

    fn move_to_top(&mut self, id: &str) {
        if let Some(pos) = self.conversations.iter().position(|c| c.id == id) {
            let conversation = self.conversations.remove(pos);
            self.conversations.insert(0, conversation);
        }
    }

    fn save(&self, id: &str) -> Result<()> {
        match self.get(id) {
            Some(c) if !c.messages.is_empty() => self.store.save(c),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(dir: &tempfile::TempDir) -> Workspace {
        Workspace::load(ConversationStore::new(dir.path())).unwrap()
    }

    #[test]
    fn full_turn_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = workspace(&dir);
        let (id, request) = ws.begin_turn("Hello there", &Settings::default()).unwrap();
        assert_eq!(request.messages, vec![Message::user("Hello there")]);
        assert_eq!(request.model, Settings::default().model);

        ws.append_delta(&id, "Hi");
        ws.append_delta(&id, "!");
        ws.finish_turn(&id, &StreamOutcome::Completed).unwrap();

        let reloaded = workspace(&dir);
        let c = reloaded.active().unwrap();
        assert_eq!(c.title, "Hello there");
        assert_eq!(
            c.messages,
            vec![Message::user("Hello there"), Message::assistant("Hi!")]
        );
    }

    #[test]
    fn failed_turns_are_marked_and_excluded_from_next_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = workspace(&dir);
        let (id, _) = ws.begin_turn("one", &Settings::default()).unwrap();
        ws.append_delta(&id, "partial");
        ws.finish_turn(&id, &StreamOutcome::Failed("boom".into()))
            .unwrap();
        assert_eq!(
            ws.active().unwrap().messages[1].error.as_deref(),
            Some("boom")
        );

        let (_, request) = ws.begin_turn("two", &Settings::default()).unwrap();
        assert_eq!(
            request.messages,
            vec![Message::user("one"), Message::user("two")]
        );
    }

    #[test]
    fn tool_use_attaches_an_app_call() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = workspace(&dir);
        let (id, _) = ws.begin_turn("roll", &Settings::default()).unwrap();
        let input = serde_json::json!({ "count": 2 });
        ws.record_tool_use(
            &id,
            &ToolUse {
                name: "roll_dice".into(),
                input: input.clone(),
            },
        );
        ws.finish_turn(&id, &StreamOutcome::Completed).unwrap();

        let reloaded = workspace(&dir);
        let app = reloaded.active().unwrap().messages[1].app.clone().unwrap();
        assert_eq!(app.input, input);
        assert_eq!(
            app.result["structuredContent"]["rolls"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        ws.record_tool_use(
            &id,
            &ToolUse {
                name: "nope".into(),
                input,
            },
        );
        assert!(ws.active().unwrap().messages[1].error.is_some());
    }

    #[test]
    fn empty_conversation_is_reused_and_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = workspace(&dir);
        let first = ws.new_conversation().to_owned();
        let second = ws.new_conversation().to_owned();
        assert_eq!(first, second);
        assert_eq!(ws.conversations().len(), 1);
        assert!(workspace(&dir).conversations().is_empty());
    }

    #[test]
    fn turn_moves_conversation_to_top_and_delete_reselects() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = workspace(&dir);
        let (a, _) = ws.begin_turn("a", &Settings::default()).unwrap();
        ws.new_conversation();
        let (b, _) = ws.begin_turn("b", &Settings::default()).unwrap();
        assert_eq!(ws.conversations()[0].id, b);

        ws.select(&a);
        ws.begin_turn("again", &Settings::default()).unwrap();
        assert_eq!(ws.conversations()[0].id, a);

        ws.delete(&a).unwrap();
        assert_eq!(ws.active_id(), Some(b.as_str()));
        assert_eq!(workspace(&dir).conversations().len(), 1);
    }
}
