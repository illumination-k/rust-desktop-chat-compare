//! AI chat app built with Dioxus (desktop / webview).

use std::cell::RefCell;
use std::rc::Rc;

use chat_core::mcp_app::{self, AppCall, AppHost, HostEvent};
use chat_core::stream::Canceller;
use chat_core::{ApiKeyStore, ChatService, ProviderKind, Role, Settings, StreamEvent};
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;

const STYLE: &str = include_str!("../assets/style.css");

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let window = WindowBuilder::new()
        .with_title("Chat (Dioxus)")
        .with_inner_size(LogicalSize::new(1000.0, 700.0));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .launch(App);
}

/// Shared UI state; every field is a `Copy` signal handle.
#[derive(Clone, Copy)]
struct AppState {
    service: Signal<Option<ChatService>>,
    streaming: Signal<Option<(String, Canceller)>>,
    error: Signal<Option<String>>,
    settings_open: Signal<bool>,
}

impl AppState {
    fn report<T, E: std::fmt::Display>(mut self, result: Result<T, E>) -> Option<T> {
        result.map_err(|e| self.error.set(Some(e.to_string()))).ok()
    }

    fn send(mut self, text: String) {
        if text.trim().is_empty() || self.streaming.read().is_some() {
            return;
        }
        let result = match self.service.write().as_mut() {
            Some(service) => service.send(text.trim()),
            None => return,
        };
        let Some((id, handle)) = self.report(result) else {
            return;
        };
        self.error.set(None);
        self.streaming.set(Some((id.clone(), handle.canceller)));
        let mut events = handle.events;
        spawn(async move {
            while let Some(event) = events.recv().await {
                let result = {
                    let mut service = self.service.write();
                    let Some(service) = service.as_mut() else {
                        return;
                    };
                    match event {
                        StreamEvent::Delta(delta) => {
                            service.workspace.append_delta(&id, &delta);
                            continue;
                        }
                        StreamEvent::ToolUse(call) => {
                            service.workspace.record_tool_use(&id, &call);
                            continue;
                        }
                        StreamEvent::Finished(outcome) => {
                            service.workspace.finish_turn(&id, &outcome)
                        }
                    }
                };
                self.streaming.set(None);
                self.report(result);
            }
        });
    }
}

#[component]
fn App() -> Element {
    let mut error = use_signal(|| None);
    let service = use_signal(|| {
        ChatService::load()
            .map_err(|e| error.set(Some(e.to_string())))
            .ok()
    });
    let state = use_context_provider(|| AppState {
        service,
        streaming: Signal::new(None),
        error,
        settings_open: Signal::new(false),
    });

    rsx! {
        style { {STYLE} }
        div { class: "app",
            Sidebar {}
            main {
                if let Some(message) = error() {
                    div { class: "error",
                        span { "{message}" }
                        button { class: "link", onclick: move |_| error.set(None), "✕" }
                    }
                }
                if (state.settings_open)() {
                    SettingsView {}
                } else {
                    Messages {}
                    Composer {}
                }
            }
        }
    }
}

#[component]
fn Sidebar() -> Element {
    let mut state = use_context::<AppState>();
    let service = state.service.read();
    let Some(service) = service.as_ref() else {
        return rsx! {};
    };
    let active = service.workspace.active_id().map(str::to_owned);
    let items: Vec<(String, String)> = service
        .workspace
        .conversations()
        .iter()
        .map(|c| (c.id.clone(), c.title.clone()))
        .collect();

    rsx! {
        nav { class: "sidebar",
            div { class: "toolbar",
                button {
                    onclick: move |_| {
                        if let Some(s) = state.service.write().as_mut() {
                            s.workspace.new_conversation();
                        }
                        state.settings_open.set(false);
                    },
                    "+ New chat"
                }
                button { class: "secondary", onclick: move |_| state.settings_open.set(true), "Settings" }
            }
            ul {
                for (id, title) in items {
                    li {
                        key: "{id}",
                        class: if active.as_deref() == Some(id.as_str()) { "active" },
                        span {
                            class: "title",
                            onclick: {
                                let id = id.clone();
                                move |_| {
                                    if let Some(s) = state.service.write().as_mut() {
                                        s.workspace.select(&id);
                                    }
                                    state.settings_open.set(false);
                                }
                            },
                            "{title}"
                        }
                        button {
                            class: "link",
                            title: "Delete",
                            onclick: move |_| {
                                if let Some((_, c)) = state.streaming.read().as_ref().filter(|(sid, _)| *sid == id) {
                                    c.cancel();
                                }
                                let result = state.service.write().as_mut().map(|s| s.workspace.delete(&id));
                                if let Some(result) = result {
                                    state.report(result);
                                }
                            },
                            "🗑"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Messages() -> Element {
    let state = use_context::<AppState>();
    let service = state.service.read();
    let Some(conversation) = service.as_ref().and_then(|s| s.workspace.active()) else {
        return rsx! {
            div { class: "empty", "Start a new conversation" }
        };
    };
    rsx! {
        // `column-reverse` keeps the view pinned to the bottom while tokens stream in.
        div { class: "messages",
            div { class: "messages-inner",
                for (i, message) in conversation.messages.iter().enumerate() {
                    MessageView {
                        key: "{conversation.id}-{i}",
                        id: format!("{}-{i}", conversation.id),
                        is_user: message.role == Role::User,
                        content: message.content.clone(),
                        error: message.error.clone(),
                        app: message.app.clone(),
                    }
                }
            }
        }
    }
}

/// Props are compared, so only the streaming message re-renders on each token.
#[component]
fn MessageView(
    id: String,
    is_user: bool,
    content: String,
    error: Option<String>,
    app: Option<AppCall>,
) -> Element {
    rsx! {
        div { class: if is_user { "message user" } else { "message assistant" },
            div { class: "role", if is_user { "You" } else { "Assistant" } }
            if is_user {
                div { class: "plain", "{content}" }
            } else {
                div { class: "markdown", dangerous_inner_html: chat_core::markdown::to_html(&content) }
            }
            if let Some(call) = app {
                McpApp { id: "mcp-app-{id}", call }
            }
            if let Some(error) = error {
                div { class: "message-error", "⚠ {error}" }
            }
        }
    }
}

/// Relays JSON-RPC between the iframe and Rust. Messages from the view are
/// forwarded with `dioxus.send`; messages for the view arrive with `dioxus.recv`.
const APP_BRIDGE_JS: &str = r#"
const id = await dioxus.recv();
window.addEventListener("message", (event) => {
  if (event.source === document.getElementById(id)?.contentWindow) dioxus.send(event.data);
});
while (true) {
  const message = await dioxus.recv();
  document.getElementById(id)?.contentWindow?.postMessage(message, "*");
}
"#;

/// An MCP App view in a sandboxed iframe (opaque origin); the host logic lives in Rust.
#[component]
fn McpApp(id: String, call: AppCall) -> Element {
    let state = use_context::<AppState>();
    let mut height = use_signal(|| mcp_app::INITIAL_HEIGHT);
    let host = use_hook(|| AppHost::new(call, None).map(|h| Rc::new(RefCell::new(h))));
    let Some(host) = host else {
        return rsx! {
            div { class: "message-error", "Unknown MCP App tool" }
        };
    };
    let bridge_host = host.clone();
    let bridge_id = id.clone();
    use_effect(move || {
        let host = bridge_host.clone();
        let mut eval = document::eval(APP_BRIDGE_JS);
        let _ = eval.send(&bridge_id);
        spawn(async move {
            while let Ok(message) = eval.recv::<serde_json::Value>().await {
                let reply = host.borrow_mut().handle(&message);
                for message in reply.messages {
                    let _ = eval.send(message);
                }
                for event in reply.events {
                    match event {
                        HostEvent::Resize(h) => height.set(h),
                        HostEvent::SendMessage(text) => state.send(text),
                        HostEvent::OpenLink(url) => mcp_app::open_link(&url),
                    }
                }
            }
        });
    });
    let teardown_host = host.clone();
    let teardown_id = id.clone();
    use_drop(move || {
        // Best effort: the iframe may already be gone.
        if let Some(message) = teardown_host.borrow().teardown() {
            document::eval(&format!(
                "document.getElementById({teardown_id:?})?.contentWindow?.postMessage({message}, '*')"
            ));
        }
    });
    let border = if host.borrow().prefers_border() {
        " bordered"
    } else {
        ""
    };
    rsx! {
        iframe {
            id: "{id}",
            class: "mcp-app{border}",
            "sandbox": "allow-scripts allow-forms",
            srcdoc: host.borrow().document(),
            style: "height: {height}px",
        }
    }
}

#[component]
fn Composer() -> Element {
    let state = use_context::<AppState>();
    let mut input = use_signal(String::new);
    let streaming = state.streaming.read().is_some();
    let mut submit = move || {
        let text = input();
        if state.streaming.read().is_none() && !text.trim().is_empty() {
            input.set(String::new());
            // The textarea is uncontrolled (see below), so clear the DOM value directly.
            document::eval("document.getElementById('composer').value = ''");
            state.send(text);
        }
    };
    rsx! {
        div { class: "composer",
            // Uncontrolled on purpose: binding `value` re-applies stale values while
            // typing fast (characters get dropped), since DOM updates are async over IPC.
            textarea {
                id: "composer",
                placeholder: "Message… (Enter to send, Shift+Enter for newline)",
                oninput: move |e| input.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    // Enter while an IME is composing confirms the conversion; don't send.
                    if e.key() == Key::Enter && !e.modifiers().shift() && !e.is_composing() {
                        e.prevent_default();
                        submit();
                    }
                },
            }
            if streaming {
                button {
                    class: "danger",
                    onclick: move |_| {
                        if let Some((_, c)) = state.streaming.read().as_ref() {
                            c.cancel();
                        }
                    },
                    "⏹ Stop"
                }
            } else {
                button { onclick: move |_| submit(), "Send" }
            }
        }
    }
}

#[component]
fn SettingsView() -> Element {
    let mut state = use_context::<AppState>();
    let initial = state
        .service
        .read()
        .as_ref()
        .map(|s| s.settings().clone())
        .unwrap_or_default();
    let mut draft = use_signal(|| initial);
    let mut api_key = use_signal(String::new);
    let mut api_key_set = use_signal(|| ApiKeyStore.is_set());

    let save = move |_| {
        let key = api_key();
        let key = key.trim();
        let result = if key.is_empty() {
            Ok(())
        } else {
            ApiKeyStore.set(key)
        };
        let saved = result.and_then(|()| match state.service.write().as_mut() {
            Some(s) => s.save_settings(draft()),
            None => Ok(()),
        });
        if state.report(saved).is_some() {
            state.settings_open.set(false);
        }
    };

    let Settings {
        provider,
        model,
        system_prompt,
        max_tokens,
    } = draft();
    rsx! {
        form { class: "settings", onsubmit: move |e| e.prevent_default(),
            h2 { "Settings" }
            label { "Provider" }
            select {
                onchange: move |e| {
                    let kind = ProviderKind::ALL.into_iter().find(|k| k.label() == e.value());
                    draft.write().provider = kind.unwrap_or_default();
                },
                for kind in ProviderKind::ALL {
                    option { value: kind.label(), selected: kind == provider, "{kind}" }
                }
            }
            label { "API key" }
            div {
                input {
                    r#type: "password",
                    placeholder: if api_key_set() { "•••• (saved in keychain)" } else { "sk-ant-…" },
                    oninput: move |e| api_key.set(e.value()),
                }
                if api_key_set() {
                    button {
                        class: "link",
                        r#type: "button",
                        onclick: move |_| {
                            if state.report(ApiKeyStore.delete()).is_some() {
                                api_key_set.set(false);
                            }
                        },
                        "Remove saved key"
                    }
                }
            }
            label { "Model" }
            input { initial_value: "{model}", oninput: move |e| draft.write().model = e.value() }
            label { "Max tokens" }
            input {
                r#type: "number",
                min: "1",
                initial_value: "{max_tokens}",
                oninput: move |e| {
                    if let Ok(n) = e.value().parse() {
                        draft.write().max_tokens = n;
                    }
                },
            }
            label { "System prompt" }
            textarea {
                rows: "6",
                initial_value: "{system_prompt}",
                oninput: move |e| draft.write().system_prompt = e.value(),
            }
            div {}
            div { class: "actions",
                button { r#type: "button", onclick: save, "Save" }
                button {
                    class: "secondary",
                    r#type: "button",
                    onclick: move |_| state.settings_open.set(false),
                    "Cancel"
                }
            }
        }
    }
}
