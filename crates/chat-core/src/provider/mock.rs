use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::json;

use super::{DeltaSink, Provider, ProviderError};
use crate::mcp_app::server::DICE_TOOL;
use crate::{ChatRequest, Role, ToolUse};

/// Env var overriding the per-token delay of [`MockProvider::default`] (milliseconds).
pub const MOCK_DELAY_ENV: &str = "CHAT_COMPARE_MOCK_DELAY_MS";

const DEFAULT_DELAY: Duration = Duration::from_millis(20);

/// A fixed Markdown response exercising headings, lists and code blocks.
pub const MOCK_RESPONSE: &str = r#"# Mock response

This reply is streamed by the **mock provider** at a constant rate, so every
framework is measured under the same load. 日本語の表示も確認します。

## Checklist

- Headings, *emphasis* and `inline code`
- Bulleted and numbered lists
  1. nested item one
  2. nested item two
- Fenced code blocks

```rust
fn main() {
    let greeting = "Hello, world!";
    println!("{greeting}");
}
```

> Streaming can be cancelled at any time.

That's all for now.
"#;

/// Reply streamed before the mock calls the dice tool (which has an MCP App view).
const DICE_RESPONSE: &str = "Rolling the dice for you. Try the buttons in the view below.\n";

/// A message mentioning dice makes the mock call the dice tool instead of the fixed response.
fn wants_dice(request: &ChatRequest) -> bool {
    request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .is_some_and(|m| {
            let text = m.content.to_lowercase();
            text.contains("dice") || text.contains("サイコロ")
        })
}

/// Streams a fixed response token by token without touching the network.
#[derive(Clone, Debug)]
pub struct MockProvider {
    pub response: String,
    pub delay: Duration,
}

impl Default for MockProvider {
    fn default() -> Self {
        let delay = std::env::var(MOCK_DELAY_ENV)
            .ok()
            .and_then(|ms| ms.parse().ok())
            .map_or(DEFAULT_DELAY, Duration::from_millis);
        Self {
            response: MOCK_RESPONSE.to_owned(),
            delay,
        }
    }
}

impl Provider for MockProvider {
    fn stream(
        &self,
        request: ChatRequest,
        sink: DeltaSink,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async move {
            let dice = wants_dice(&request);
            let response = if dice { DICE_RESPONSE } else { &self.response };
            for token in response.split_inclusive(char::is_whitespace) {
                if !self.delay.is_zero() {
                    tokio::time::sleep(self.delay).await;
                }
                sink.send(token);
            }
            if dice {
                sink.tool_use(ToolUse {
                    name: DICE_TOOL.to_owned(),
                    input: json!({ "sides": 6, "count": 3 }),
                });
            }
            Ok(())
        })
    }
}
