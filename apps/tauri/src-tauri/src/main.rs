//! Tauri v2 backend: owns chat-core state and exposes it to the web frontend
//! through commands (request/response) and the `stream` event (token push).

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use chat_core::stream::Canceller;
use chat_core::{ApiKeyStore, ChatService, Role, Settings, StreamEvent, markdown};
use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _, State};

struct Inner {
    service: ChatService,
    streaming: Option<(String, Canceller)>,
}

type AppState = Mutex<Inner>;
type CommandResult<T> = Result<T, String>;

#[derive(Serialize)]
struct ConversationItem {
    id: String,
    title: String,
}

#[derive(Serialize)]
struct MessageView {
    role: Role,
    /// Plain text for user messages, sanitized HTML for assistant messages.
    content: String,
    error: Option<String>,
}

/// Everything the frontend renders, sent after each state change.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    conversations: Vec<ConversationItem>,
    active_id: Option<String>,
    messages: Vec<MessageView>,
    streaming: bool,
}

/// Payload of the `stream` event: the re-rendered HTML of the streaming message.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamPayload {
    conversation_id: String,
    html: String,
    done: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    settings: Settings,
    api_key_set: bool,
}

fn view_message(message: &chat_core::Message) -> MessageView {
    MessageView {
        role: message.role,
        content: match message.role {
            Role::User => message.content.clone(),
            Role::Assistant => markdown::to_html(&message.content),
        },
        error: message.error.clone(),
    }
}

fn snapshot(inner: &Inner) -> Snapshot {
    let workspace = &inner.service.workspace;
    Snapshot {
        conversations: workspace
            .conversations()
            .iter()
            .map(|c| ConversationItem {
                id: c.id.clone(),
                title: c.title.clone(),
            })
            .collect(),
        active_id: workspace.active_id().map(str::to_owned),
        messages: workspace
            .active()
            .map(|c| c.messages.iter().map(view_message).collect())
            .unwrap_or_default(),
        streaming: inner.streaming.is_some(),
    }
}

fn lock(state: &AppState) -> std::sync::MutexGuard<'_, Inner> {
    // A panic while holding the lock cannot leave `Inner` half-updated in a harmful way.
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> Snapshot {
    snapshot(&lock(&state))
}

#[tauri::command]
fn new_chat(state: State<'_, AppState>) -> Snapshot {
    let mut inner = lock(&state);
    inner.service.workspace.new_conversation();
    snapshot(&inner)
}

#[tauri::command]
fn select(state: State<'_, AppState>, id: String) -> Snapshot {
    let mut inner = lock(&state);
    inner.service.workspace.select(&id);
    snapshot(&inner)
}

#[tauri::command]
fn delete(state: State<'_, AppState>, id: String) -> CommandResult<Snapshot> {
    let mut inner = lock(&state);
    if let Some((_, canceller)) = inner.streaming.as_ref().filter(|(sid, _)| *sid == id) {
        canceller.cancel();
    }
    inner
        .service
        .workspace
        .delete(&id)
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&inner))
}

/// Async so it runs on Tauri's tokio runtime, which `ChatService::send` spawns onto.
#[tauri::command]
async fn send(app: AppHandle, state: State<'_, AppState>, text: String) -> CommandResult<Snapshot> {
    let mut inner = lock(&state);
    let text = text.trim();
    if text.is_empty() || inner.streaming.is_some() {
        return Ok(snapshot(&inner));
    }
    let (id, handle) = inner.service.send(text).map_err(|e| e.to_string())?;
    inner.streaming = Some((id.clone(), handle.canceller));
    let mut events = handle.events;
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let state = app.state::<AppState>();
            let mut inner = lock(&state);
            let done = match event {
                StreamEvent::Delta(delta) => {
                    inner.service.workspace.append_delta(&id, &delta);
                    false
                }
                StreamEvent::Finished(outcome) => {
                    if let Err(e) = inner.service.workspace.finish_turn(&id, &outcome) {
                        tracing::error!(%e, "failed to save conversation");
                    }
                    inner.streaming = None;
                    true
                }
            };
            let html = inner
                .service
                .workspace
                .get(&id)
                .and_then(|c| c.messages.last())
                .map(|m| markdown::to_html(&m.content))
                .unwrap_or_default();
            drop(inner);
            let _ = app.emit(
                "stream",
                StreamPayload {
                    conversation_id: id.clone(),
                    html,
                    done,
                },
            );
        }
    });
    Ok(snapshot(&inner))
}

#[tauri::command]
fn stop(state: State<'_, AppState>) {
    if let Some((_, canceller)) = &lock(&state).streaming {
        canceller.cancel();
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    SettingsView {
        settings: lock(&state).service.settings().clone(),
        api_key_set: ApiKeyStore.is_set(),
    }
}

/// `api_key`: `None` or empty keeps the stored key.
#[tauri::command]
fn save_settings(
    state: State<'_, AppState>,
    settings: Settings,
    api_key: Option<String>,
) -> CommandResult<()> {
    if let Some(key) = api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        ApiKeyStore.set(key).map_err(|e| e.to_string())?;
    }
    lock(&state)
        .service
        .save_settings(settings)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_api_key() -> CommandResult<()> {
    ApiKeyStore.delete().map_err(|e| e.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let service = ChatService::load()?;
    tauri::Builder::default()
        .manage(Mutex::new(Inner {
            service,
            streaming: None,
        }))
        .invoke_handler(tauri::generate_handler![
            get_state,
            new_chat,
            select,
            delete,
            send,
            stop,
            get_settings,
            save_settings,
            remove_api_key,
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
