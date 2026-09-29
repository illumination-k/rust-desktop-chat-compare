//! An in-process stand-in for an MCP server that exposes one tool with a UI.
//!
//! It keeps the comparison network-free, like the mock provider.

use serde_json::{Value, json};

/// Tool name the mock provider calls when asked to roll dice.
pub const DICE_TOOL: &str = "roll_dice";
const DICE_URI: &str = "ui://demo/dice";
const DICE_HTML: &str = include_str!("../../assets/dice-app.html");
const MAX_DICE: u64 = 20;

/// Tool definition as `tools/list` would return it, or `None` for unknown tools.
pub fn tool(name: &str) -> Option<Value> {
    (name == DICE_TOOL).then(|| {
        json!({
            "name": DICE_TOOL,
            "description": "Roll dice and show them in an interactive view",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sides": { "type": "integer", "minimum": 2 },
                    "count": { "type": "integer", "minimum": 1, "maximum": MAX_DICE }
                }
            },
            "_meta": { "ui": { "resourceUri": DICE_URI, "visibility": ["model", "app"] } }
        })
    })
}

/// Runs a tool and returns its `CallToolResult`.
pub fn call_tool(name: &str, arguments: &Value) -> Result<Value, String> {
    if name != DICE_TOOL {
        return Err(format!("Unknown tool: {name}"));
    }
    let arg = |key, default| {
        arguments
            .get(key)
            .and_then(Value::as_u64)
            .unwrap_or(default)
    };
    let sides = arg("sides", 6).max(2);
    let count = arg("count", 1).clamp(1, MAX_DICE);
    let rolls: Vec<u64> = (0..count).map(|_| fastrand::u64(1..=sides)).collect();
    let text = rolls
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Ok(json!({
        "content": [{ "type": "text", "text": format!("Rolled {text}") }],
        "structuredContent": { "rolls": rolls }
    }))
}

/// `resources/read` result for a UI resource, or `None` if it does not exist.
pub fn read_resource(uri: &str) -> Option<Value> {
    (uri == DICE_URI).then(|| {
        json!({
            "contents": [{
                "uri": DICE_URI,
                "mimeType": super::RESOURCE_MIME_TYPE,
                "text": DICE_HTML,
                "_meta": { "ui": { "prefersBorder": true } }
            }]
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dice_rolls_are_within_range() {
        let result = call_tool(DICE_TOOL, &json!({ "sides": 4, "count": 50 })).unwrap();
        let rolls = result["structuredContent"]["rolls"].as_array().unwrap();
        assert_eq!(rolls.len(), 20);
        assert!(rolls.iter().all(|r| (1..=4).contains(&r.as_u64().unwrap())));
    }

    #[test]
    fn unknown_tools_and_resources_are_rejected() {
        assert!(call_tool("nope", &json!({})).is_err());
        assert!(tool("nope").is_none());
        assert!(read_resource("ui://nope").is_none());
    }
}
