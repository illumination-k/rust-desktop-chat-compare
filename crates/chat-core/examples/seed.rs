//! Writes a long conversation for the "1,000 messages" memory/scroll benchmark.
//!
//! ```sh
//! CHAT_COMPARE_HOME=/tmp/chat-bench cargo run -p chat-core --example seed -- 1000
//! ```

use chat_core::provider::MockProvider;
use chat_core::{AppPaths, Conversation, ConversationStore, Message};

fn main() -> chat_core::Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(1000);
    let paths = AppPaths::from_env()?;
    let reply = MockProvider::default().response;
    let mut conversation = Conversation::new();
    conversation.title = format!("Benchmark ({count} messages)");
    conversation.messages = (0..count)
        .map(|i| {
            if i % 2 == 0 {
                Message::user(format!("Question #{}", i / 2 + 1))
            } else {
                Message::assistant(reply.clone())
            }
        })
        .collect();
    ConversationStore::new(&paths.conversations_dir).save(&conversation)?;
    tracing::info!(dir = %paths.conversations_dir.display(), "seeded");
    Ok(())
}
