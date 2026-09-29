//! AI chat app built with Slint. The UI lives in `ui/app.slint`; this file wires it to chat-core.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use app_webview::{AppWebView, Bounds};
use chat_core::markdown::{self, Block, Span};
use chat_core::mcp_app::{self, AppHost, HostEvent};
use chat_core::stream::Canceller;
use chat_core::{ApiKeyStore, ChatService, ProviderKind, Role, Settings, StreamEvent};
use slint::{ComponentHandle as _, Model as _, ModelRc, SharedString, StyledText, VecModel};

mod ui {
    // Generated code does not follow the workspace lint policy.
    #![allow(
        clippy::all,
        clippy::pedantic,
        clippy::unwrap_used,
        clippy::expect_used
    )]
    slint::include_modules!();
}
use ui::{AppWindow, BlockKind, ChatMessage, ConversationItem, MdBlock};

struct State {
    service: ChatService,
    streaming: Option<(String, Canceller)>,
    apps: AppViews,
}

/// MCP App views (child webviews) of the active conversation, keyed by
/// `conversation id:message index`.
#[derive(Default)]
struct AppViews {
    supported: bool,
    views: HashMap<String, AppView>,
    /// Last reported placeholder rect (window coordinates) and the list's `content-y` then.
    slots: HashMap<String, (Bounds, f64)>,
    /// Visible area of the message list and its current `content-y`.
    list: Option<(Bounds, f64)>,
}

struct AppView {
    webview: AppWebView,
    height: f32,
}

type Shared = Rc<RefCell<State>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    // Slint owns the main thread; streams run on this runtime and are consumed via `spawn_local`.
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();

    let ui = AppWindow::new()?;
    let supported = app_webview::init();
    ui.set_app_views_supported(supported);
    let state: Shared = Rc::new(RefCell::new(State {
        service: ChatService::load()?,
        streaming: None,
        apps: AppViews {
            supported,
            ..AppViews::default()
        },
    }));
    let messages = Rc::new(VecModel::<ChatMessage>::default());
    ui.set_messages(messages.clone().into());
    ui.set_providers(ModelRc::new(VecModel::from(
        ProviderKind::ALL
            .map(|k| SharedString::from(k.label()))
            .to_vec(),
    )));
    sync_all(&ui, &state.borrow(), &messages);

    // Wraps a callback body with access to the window, state and message model.
    macro_rules! handler {
        (|$ui:ident, $state:ident, $messages:ident $(, $arg:ident)*| $body:block) => {{
            let weak = ui.as_weak();
            let state = state.clone();
            let messages = messages.clone();
            move |$($arg),*| {
                let Some($ui) = weak.upgrade() else { return };
                let $state = &state;
                let $messages = &messages;
                $body
            }
        }};
    }

    ui.on_new_chat(handler!(|ui, state, messages| {
        state.borrow_mut().service.workspace.new_conversation();
        ui.set_settings_open(false);
        sync_all(&ui, &state.borrow(), messages);
    }));
    ui.on_select(handler!(|ui, state, messages, id| {
        state.borrow_mut().service.workspace.select(&id);
        ui.set_settings_open(false);
        sync_all(&ui, &state.borrow(), messages);
    }));
    ui.on_delete(handler!(|ui, state, messages, id| {
        let mut st = state.borrow_mut();
        if let Some((_, canceller)) = st.streaming.as_ref().filter(|(sid, _)| *sid == id.as_str()) {
            canceller.cancel();
        }
        report(&ui, st.service.workspace.delete(&id));
        sync_all(&ui, &st, messages);
    }));
    ui.on_stop(handler!(|_ui, state, _messages| {
        if let Some((_, canceller)) = &state.borrow().streaming {
            canceller.cancel();
        }
    }));
    ui.on_send(handler!(|ui, state, messages| {
        send(&ui, state, messages)
    }));
    ui.on_dismiss_error(handler!(|ui, _state, _messages| {
        ui.set_error(SharedString::new())
    }));

    ui.on_open_settings(handler!(|ui, state, _messages| {
        let settings = state.borrow().service.settings().clone();
        let index = ProviderKind::ALL
            .iter()
            .position(|k| *k == settings.provider);
        ui.set_provider_index(
            index
                .and_then(|i| i32::try_from(i).ok())
                .unwrap_or_default(),
        );
        ui.set_api_key(SharedString::new());
        ui.set_api_key_set(ApiKeyStore.is_set());
        ui.set_model(settings.model.into());
        ui.set_max_tokens(i32::try_from(settings.max_tokens).unwrap_or(i32::MAX));
        ui.set_system_prompt(settings.system_prompt.into());
        ui.set_settings_open(true);
    }));
    ui.on_remove_key(handler!(|ui, _state, _messages| {
        if report(&ui, ApiKeyStore.delete()) {
            ui.set_api_key_set(false);
        }
    }));
    ui.on_cancel_settings(handler!(|ui, _state, _messages| {
        ui.set_settings_open(false)
    }));
    ui.on_save_settings(handler!(|ui, state, _messages| {
        let provider = usize::try_from(ui.get_provider_index())
            .ok()
            .and_then(|i| ProviderKind::ALL.get(i).copied())
            .unwrap_or_default();
        let settings = Settings {
            provider,
            model: ui.get_model().trim().to_owned(),
            system_prompt: ui.get_system_prompt().into(),
            max_tokens: u32::try_from(ui.get_max_tokens()).unwrap_or(1),
        };
        let key = ui.get_api_key();
        let key = key.trim();
        let result = if key.is_empty() {
            Ok(())
        } else {
            ApiKeyStore.set(key)
        };
        if report(
            &ui,
            result.and_then(|()| state.borrow_mut().service.save_settings(settings)),
        ) {
            ui.set_settings_open(false);
        }
    }));

    ui.on_app_geometry(handler!(|_ui,
                                 state,
                                 _messages,
                                 key,
                                 x,
                                 y,
                                 width,
                                 height,
                                 content_y| {
        let bounds = rect(x, y, width, height);
        state
            .borrow_mut()
            .apps
            .slots
            .insert(key.into(), (bounds, f64::from(content_y)));
    }));
    ui.on_list_geometry(handler!(|_ui,
                                  state,
                                  _messages,
                                  x,
                                  y,
                                  width,
                                  height,
                                  content_y| {
        state.borrow_mut().apps.list = Some((rect(x, y, width, height), f64::from(content_y)));
    }));
    ui.on_app_tick(handler!(|ui, state, messages| {
        app_tick(&ui, state, messages)
    }));

    ui.run()?;
    Ok(())
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
    Bounds {
        x: f64::from(x),
        y: f64::from(y),
        width: f64::from(width),
        height: f64::from(height),
    }
}

fn app_key(conversation: &chat_core::Conversation, index: usize) -> String {
    format!("{}:{index}", conversation.id)
}

/// Pumps the webviews, creates/drops them to match the conversation and lays
/// them over their placeholders.
fn app_tick(ui: &AppWindow, state: &Shared, messages: &Rc<VecModel<ChatMessage>>) {
    app_webview::pump_platform();
    let mut send_text = None;
    let mut resized = false;
    {
        let mut st = state.borrow_mut();
        let State { service, apps, .. } = &mut *st;
        let conversation = service.workspace.active();
        let keys: Vec<String> = conversation
            .map(|c| {
                c.messages
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.app.is_some())
                    .map(|(i, _)| app_key(c, i))
                    .collect()
            })
            .unwrap_or_default();
        apps.views.retain(|key, _| keys.contains(key));
        apps.slots.retain(|key, _| keys.contains(key));
        if apps.supported {
            for key in &keys {
                if apps.views.contains_key(key) {
                    continue;
                }
                let host = conversation
                    .zip(
                        key.rsplit_once(':')
                            .and_then(|(_, i)| i.parse::<usize>().ok()),
                    )
                    .and_then(|(c, i)| c.messages.get(i)?.app.clone())
                    .and_then(|app| AppHost::new(app, None));
                let Some(host) = host else { continue };
                match AppWebView::new(&ui.window().window_handle(), host, || {}) {
                    Ok(webview) => {
                        let height = mcp_app::INITIAL_HEIGHT as f32;
                        apps.views.insert(key.clone(), AppView { webview, height });
                    }
                    Err(e) => {
                        ui.set_error(format!("Failed to create MCP App view: {e}").into());
                        apps.supported = false;
                    }
                }
            }
        }
        let settings_open = ui.get_settings_open();
        for (key, view) in &mut apps.views {
            for event in view.webview.pump() {
                match event {
                    HostEvent::Resize(height) => {
                        resized |= (view.height - height as f32).abs() >= 1.0;
                        view.height = height as f32;
                    }
                    HostEvent::SendMessage(text) => send_text = Some(text),
                    HostEvent::OpenLink(url) => mcp_app::open_link(&url),
                }
            }
            match (apps.list, apps.slots.get(key)) {
                (Some((viewport, list_y)), Some(&(slot, slot_y))) if !settings_open => {
                    // The slot may be stale (virtualized away); shift it by the scroll since.
                    let placeholder = Bounds {
                        y: slot.y + list_y - slot_y,
                        ..slot
                    };
                    view.webview.place(placeholder, viewport);
                }
                _ => view.webview.hide(),
            }
        }
    }
    if resized {
        sync_all(ui, &state.borrow(), messages);
    }
    if let Some(text) = send_text {
        ui.set_input_text(text.into());
        send(ui, state, messages);
    }
}

fn send(ui: &AppWindow, state: &Shared, messages: &Rc<VecModel<ChatMessage>>) {
    let text = ui.get_input_text().trim().to_owned();
    if text.is_empty() || state.borrow().streaming.is_some() {
        return;
    }
    let result = state.borrow_mut().service.send(&text);
    let (id, handle) = match result {
        Ok(ok) => ok,
        Err(e) => {
            report(ui, Err::<(), _>(e));
            return;
        }
    };
    ui.set_input_text(SharedString::new());
    ui.set_error(SharedString::new());
    state.borrow_mut().streaming = Some((id.clone(), handle.canceller));
    sync_all(ui, &state.borrow(), messages);

    let weak = ui.as_weak();
    let state = state.clone();
    let messages = messages.clone();
    let mut events = handle.events;
    // tokio's channel works on any executor, so the UI thread awaits it directly.
    let task = slint::spawn_local(async move {
        while let Some(event) = events.recv().await {
            let Some(ui) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            match event {
                StreamEvent::Delta(delta) => st.service.workspace.append_delta(&id, &delta),
                StreamEvent::ToolUse(call) => {
                    st.service.workspace.record_tool_use(&id, &call);
                    ui.set_app_views_active(true);
                }
                StreamEvent::Finished(outcome) => {
                    report(&ui, st.service.workspace.finish_turn(&id, &outcome));
                    st.streaming = None;
                    ui.set_streaming(false);
                }
            }
            if st.service.workspace.active_id() == Some(id.as_str()) {
                sync_last(&st, &messages);
            }
        }
    });
    report(ui, task.map(drop));
}

/// Shows `result`'s error in the banner; returns whether it succeeded.
fn report<E: std::fmt::Display>(ui: &AppWindow, result: Result<(), E>) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            ui.set_error(e.to_string().into());
            false
        }
    }
}

fn sync_all(ui: &AppWindow, state: &State, messages: &VecModel<ChatMessage>) {
    let workspace = &state.service.workspace;
    let items: Vec<ConversationItem> = workspace
        .conversations()
        .iter()
        .map(|c| ConversationItem {
            id: c.id.as_str().into(),
            title: c.title.as_str().into(),
        })
        .collect();
    ui.set_conversations(ModelRc::new(VecModel::from(items)));
    ui.set_active_id(workspace.active_id().unwrap_or_default().into());
    ui.set_streaming(state.streaming.is_some());
    let rows: Vec<ChatMessage> = workspace
        .active()
        .map(|c| {
            (0..c.messages.len())
                .map(|i| to_chat_message(c, i, &state.apps))
                .collect()
        })
        .unwrap_or_default();
    ui.set_app_views_active(
        rows.iter().any(|m| !m.app_key.is_empty()) || !state.apps.views.is_empty(),
    );
    messages.set_vec(rows);
}

/// Re-renders only the last (streaming) message.
fn sync_last(state: &State, messages: &VecModel<ChatMessage>) {
    let Some(conversation) = state.service.workspace.active() else {
        return;
    };
    if let (Some(last), Some(index)) = (
        messages.row_count().checked_sub(1),
        conversation.messages.len().checked_sub(1),
    ) {
        messages.set_row_data(last, to_chat_message(conversation, index, &state.apps));
    }
}

fn to_chat_message(
    conversation: &chat_core::Conversation,
    index: usize,
    apps: &AppViews,
) -> ChatMessage {
    let message = &conversation.messages[index];
    let is_user = message.role == Role::User;
    let app_key = if message.app.is_some() {
        app_key(conversation, index)
    } else {
        String::new()
    };
    let app_height = apps
        .views
        .get(&app_key)
        .map_or(mcp_app::INITIAL_HEIGHT as f32, |v| v.height);
    let blocks: Vec<MdBlock> = if is_user {
        Vec::new()
    } else {
        markdown::parse(&message.content)
            .iter()
            .map(to_md_block)
            .collect()
    };
    ChatMessage {
        is_user,
        plain: message.content.as_str().into(),
        blocks: ModelRc::new(VecModel::from(blocks)),
        error: message.error.as_deref().unwrap_or_default().into(),
        app_key: app_key.into(),
        app_height,
    }
}

fn to_md_block(block: &Block) -> MdBlock {
    let mut out = MdBlock {
        kind: BlockKind::Paragraph,
        text: StyledText::default(),
        code: SharedString::new(),
        lang: SharedString::new(),
        level: 0,
        depth: 0,
        marker: SharedString::new(),
    };
    match block {
        Block::Heading { level, spans } => {
            out.kind = BlockKind::Heading;
            out.level = i32::from(*level);
            out.text = styled(spans, true);
        }
        Block::Paragraph(spans) => out.text = styled(spans, false),
        Block::ListItem {
            depth,
            number,
            spans,
        } => {
            out.kind = BlockKind::ListItem;
            out.depth = i32::try_from(*depth).unwrap_or_default();
            out.marker = number
                .map_or_else(|| "•".to_owned(), |n| format!("{n}."))
                .into();
            out.text = styled(spans, false);
        }
        Block::Quote(spans) => {
            out.kind = BlockKind::Quote;
            out.text = styled(spans, false);
        }
        Block::CodeBlock { lang, code } => {
            out.kind = BlockKind::Code;
            out.lang = lang.as_str().into();
            out.code = code.as_str().into();
        }
        Block::Rule => out.kind = BlockKind::Rule,
    }
    out
}

/// Slint's `StyledText` understands inline Markdown only, so spans are re-encoded
/// as escaped inline Markdown (block structure is handled by the `.slint` side).
fn styled(spans: &[Span], force_bold: bool) -> StyledText {
    let mut source = String::new();
    for span in spans {
        let body = if span.code {
            code_span(&span.text)
        } else {
            escape(&span.text)
        };
        let marker = match (span.bold || force_bold, span.italic) {
            (true, true) => "***",
            (true, false) => "**",
            (false, true) => "*",
            (false, false) => "",
        };
        let trimmed = body.trim();
        if marker.is_empty() || trimmed.is_empty() {
            source.push_str(&body);
        } else {
            // Emphasis delimiters must hug non-whitespace, so keep outer spaces outside.
            let lead = &body[..body.len() - body.trim_start().len()];
            let trail = &body[body.trim_end().len()..];
            source.push_str(&format!("{lead}{marker}{trimmed}{marker}{trail}"));
        }
    }
    StyledText::from_markdown(&source).unwrap_or_else(|_| {
        let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
        StyledText::from_plain_text(&plain)
    })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn code_span(text: &str) -> String {
    let longest_run = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest_run + 1);
    format!("{fence} {text} {fence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markdown_punctuation() {
        assert_eq!(escape("a*b_<c>"), r"a\*b\_\<c\>");
    }

    #[test]
    fn code_span_fence_outgrows_inner_backticks() {
        assert_eq!(code_span("a``b"), "``` a``b ```");
    }
}
