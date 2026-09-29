//! Host side of MCP Apps (SEP-1865, spec version 2026-01-26): interactive
//! HTML views that tools attach to their results, rendered inside the chat.
//!
//! [`AppHost`] is transport-agnostic. A UI loads [`AppHost::document`] into a
//! webview or a sandboxed iframe, feeds every JSON-RPC message from the view to
//! [`AppHost::handle`], posts the returned messages back, and reacts to the
//! returned [`HostEvent`]s.
//!
//! <https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx>

mod csp;
pub mod server;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2026-01-26";
pub const RESOURCE_MIME_TYPE: &str = "text/html;profile=mcp-app";
/// Views may grow up to this height (logical pixels) inside a message.
pub const MAX_HEIGHT: f64 = 600.0;
/// Height to reserve before the view reports its size.
pub const INITIAL_HEIGHT: f64 = 160.0;

const STYLE_VARIABLES: &[(&str, &str)] = &[
    ("--color-background-primary", "light-dark(#ffffff, #171717)"),
    (
        "--color-background-secondary",
        "light-dark(#f5f5f5, #262626)",
    ),
    ("--color-text-primary", "light-dark(#171717, #fafafa)"),
    ("--color-text-secondary", "light-dark(#525252, #a3a3a3)"),
    ("--color-border-primary", "light-dark(#d4d4d4, #404040)"),
    (
        "--font-sans",
        "system-ui, -apple-system, 'Segoe UI', sans-serif",
    ),
    (
        "--font-mono",
        "ui-monospace, SFMono-Regular, Menlo, monospace",
    ),
    ("--border-radius-md", "8px"),
];

/// A tool call whose result is rendered by an MCP App view. Stored on the assistant message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCall {
    pub tool: String,
    pub input: Value,
    /// The `CallToolResult`.
    pub result: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

/// Something the UI has to do on behalf of the view.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum HostEvent {
    /// Open an http(s) URL in the browser (see [`open_link`]).
    OpenLink(String),
    /// Send this text as the next user message (`ui/message`).
    SendMessage(String),
    /// The view's content height, already clamped to [`MAX_HEIGHT`].
    Resize(f64),
}

/// Messages to post to the view (in order) and events for the UI.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Reply {
    pub messages: Vec<Value>,
    pub events: Vec<HostEvent>,
}

/// One rendered view: answers its requests and pushes the tool data to it.
#[derive(Debug)]
pub struct AppHost {
    call: AppCall,
    tool: Value,
    html: String,
    ui_meta: Value,
    theme: Option<Theme>,
    initialized: bool,
}

impl AppHost {
    /// Resolves the tool's UI resource. `None` if the tool has no UI.
    pub fn new(call: AppCall, theme: Option<Theme>) -> Option<Self> {
        let tool = server::tool(&call.tool)?;
        let uri = tool.pointer("/_meta/ui/resourceUri")?.as_str()?;
        let resource = server::read_resource(uri)?;
        let content = resource.pointer("/contents/0")?;
        Some(Self {
            html: content.get("text")?.as_str()?.to_owned(),
            ui_meta: content.pointer("/_meta/ui").cloned().unwrap_or(Value::Null),
            call,
            tool,
            theme,
            initialized: false,
        })
    }

    /// The view's HTML with its CSP applied.
    pub fn document(&self) -> String {
        let csp = self.ui_meta.get("csp").unwrap_or(&Value::Null);
        csp::inject(&self.html, &csp::build(csp))
    }

    /// Whether the view asked for a visible border (`_meta.ui.prefersBorder`).
    pub fn prefers_border(&self) -> bool {
        self.ui_meta.get("prefersBorder").and_then(Value::as_bool) != Some(false)
    }

    /// Handles one message from the view (a JSON string, as sent over IPC).
    pub fn handle_json(&mut self, raw: &str) -> Reply {
        serde_json::from_str(raw).map_or_else(|_| Reply::default(), |msg| self.handle(&msg))
    }

    /// Handles one JSON-RPC message from the view.
    pub fn handle(&mut self, msg: &Value) -> Reply {
        let mut reply = Reply::default();
        if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return reply;
        }
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response to our teardown request; nothing waits for it.
            return reply;
        };
        let params = msg.get("params").unwrap_or(&Value::Null);
        match msg.get("id") {
            Some(id) => {
                let response = match self.request(method, params, &mut reply.events) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err((code, message)) => json!({
                        "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message }
                    }),
                };
                reply.messages.push(response);
            }
            None => self.notification(method, params, &mut reply),
        }
        reply
    }

    /// Best-effort `ui/resource-teardown` to post before the view is destroyed.
    pub fn teardown(&self) -> Option<Value> {
        self.initialized.then(|| {
            json!({
                "jsonrpc": "2.0", "id": "teardown", "method": "ui/resource-teardown",
                "params": { "reason": "The message is no longer displayed" }
            })
        })
    }

    fn notification(&mut self, method: &str, params: &Value, reply: &mut Reply) {
        match method {
            "ui/notifications/initialized" if !self.initialized => {
                self.initialized = true;
                reply.messages.push(notification(
                    "ui/notifications/tool-input",
                    json!({ "arguments": self.call.input }),
                ));
                reply.messages.push(notification(
                    "ui/notifications/tool-result",
                    self.call.result.clone(),
                ));
            }
            "ui/notifications/size-changed" => {
                if let Some(height) = params.get("height").and_then(Value::as_f64) {
                    reply
                        .events
                        .push(HostEvent::Resize(height.clamp(0.0, MAX_HEIGHT)));
                }
            }
            "notifications/message" => {
                tracing::info!(target: "mcp_app", data = %params, "view log")
            }
            _ => {}
        }
    }

    fn request(
        &self,
        method: &str,
        params: &Value,
        events: &mut Vec<HostEvent>,
    ) -> Result<Value, (i64, String)> {
        let str_param = |key: &str| params.get(key).and_then(Value::as_str);
        match method {
            "ui/initialize" => Ok(self.initialize_result()),
            "ping" | "ui/update-model-context" => Ok(json!({})),
            "ui/request-display-mode" => Ok(json!({ "mode": "inline" })),
            "ui/open-link" => {
                let url = str_param("url")
                    .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                    .ok_or((-32000, "Invalid URL".to_owned()))?;
                events.push(HostEvent::OpenLink(url.to_owned()));
                Ok(json!({}))
            }
            "ui/message" => {
                let text = params
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .filter(|t| !t.trim().is_empty())
                    .ok_or((-32000, "Invalid message format".to_owned()))?;
                events.push(HostEvent::SendMessage(text.to_owned()));
                Ok(json!({}))
            }
            "tools/call" => {
                let name = str_param("name").unwrap_or_default();
                let callable = server::tool(name)
                    .and_then(|t| t.pointer("/_meta/ui/visibility").cloned())
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
                    .is_none_or(|v| v.iter().any(|s| s == "app"));
                if !callable {
                    return Err((-32000, format!("Tool {name} is not callable by apps")));
                }
                let arguments = params.get("arguments").unwrap_or(&Value::Null);
                server::call_tool(name, arguments).map_err(|e| (-32602, e))
            }
            "resources/read" => {
                let uri = str_param("uri").unwrap_or_default();
                server::read_resource(uri).ok_or((-32002, format!("Resource not found: {uri}")))
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        }
    }

    fn initialize_result(&self) -> Value {
        let variables: serde_json::Map<String, Value> = STYLE_VARIABLES
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect();
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "hostInfo": { "name": "chat-compare", "version": env!("CARGO_PKG_VERSION") },
            "hostCapabilities": {
                "openLinks": {},
                "serverTools": {},
                "serverResources": {},
                "logging": {},
                "sandbox": {
                    "csp": self.ui_meta.get("csp"),
                    "permissions": self.ui_meta.get("permissions")
                }
            },
            "hostContext": {
                "toolInfo": { "tool": self.tool },
                "theme": self.theme,
                "styles": { "variables": variables },
                "displayMode": "inline",
                "availableDisplayModes": ["inline"],
                "containerDimensions": { "maxHeight": MAX_HEIGHT },
                "platform": "desktop",
                "userAgent": "chat-compare"
            }
        })
    }
}

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// Opens a URL from [`HostEvent::OpenLink`] in the default browser.
pub fn open_link(url: &str) {
    if let Err(e) = webbrowser::open(url) {
        tracing::warn!("failed to open {url}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> AppHost {
        let input = json!({ "count": 2 });
        let result = server::call_tool(server::DICE_TOOL, &input).unwrap();
        AppHost::new(
            AppCall {
                tool: server::DICE_TOOL.into(),
                input,
                result,
            },
            Some(Theme::Dark),
        )
        .unwrap()
    }

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn document_has_csp_before_markup() {
        let doc = host().document();
        assert!(doc.starts_with("<!doctype html><meta http-equiv=\"Content-Security-Policy\""));
    }

    #[test]
    fn handshake_then_tool_data() {
        let mut host = host();
        assert!(host.teardown().is_none());
        let reply = host.handle(&request(1, "ui/initialize", json!({})));
        let result = &reply.messages[0]["result"];
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["hostContext"]["theme"], "dark");
        assert_eq!(
            result["hostContext"]["toolInfo"]["tool"]["name"],
            "roll_dice"
        );

        let reply =
            host.handle_json(r#"{"jsonrpc":"2.0","method":"ui/notifications/initialized"}"#);
        let methods: Vec<_> = reply.messages.iter().map(|m| m["method"].clone()).collect();
        assert_eq!(
            methods,
            [
                "ui/notifications/tool-input",
                "ui/notifications/tool-result"
            ]
        );
        assert_eq!(reply.messages[0]["params"]["arguments"]["count"], 2);
        assert!(host.teardown().is_some());
    }

    #[test]
    fn requests_become_events() {
        let mut host = host();
        let reply = host.handle(&request(
            1,
            "ui/open-link",
            json!({ "url": "https://example.com" }),
        ));
        assert_eq!(
            reply.events,
            [HostEvent::OpenLink("https://example.com".into())]
        );
        let reply = host.handle(&request(
            2,
            "ui/open-link",
            json!({ "url": "file:///etc/passwd" }),
        ));
        assert_eq!(reply.messages[0]["error"]["message"], "Invalid URL");

        let text = json!({ "role": "user", "content": { "type": "text", "text": "hi" } });
        let reply = host.handle(&request(3, "ui/message", text));
        assert_eq!(reply.events, [HostEvent::SendMessage("hi".into())]);

        let size = json!({ "jsonrpc": "2.0", "method": "ui/notifications/size-changed", "params": { "width": 1, "height": 9999 } });
        assert_eq!(host.handle(&size).events, [HostEvent::Resize(MAX_HEIGHT)]);
    }

    #[test]
    fn tools_call_runs_the_tool_and_unknown_methods_fail() {
        let mut host = host();
        let params = json!({ "name": "roll_dice", "arguments": { "count": 3 } });
        let reply = host.handle(&request(1, "tools/call", params));
        let rolls = &reply.messages[0]["result"]["structuredContent"]["rolls"];
        assert_eq!(rolls.as_array().unwrap().len(), 3);

        let reply = host.handle(&request(2, "nope", json!({})));
        assert_eq!(reply.messages[0]["error"]["code"], -32601);
    }
}
