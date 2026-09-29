//! AI chat app built with iced (Elm architecture).

mod markdown;

use std::collections::HashMap;
use std::time::Duration;

use app_webview::{AppWebView, Bounds, ParentWindow};
use chat_core::markdown::Block;
use chat_core::mcp_app::{self, AppHost, HostEvent};
use chat_core::stream::Canceller;
use chat_core::{ApiKeyStore, ChatService, ProviderKind, Role, Settings, StreamEvent};
use iced::futures::stream;
use iced::keyboard::{Key, key::Named};
use iced::widget::selector::{self, Candidate};
use iced::widget::{
    self, button, column, container, pick_list, row, rule, scrollable, space, text, text_editor,
    text_input,
};
use iced::{Element, Fill, Length, Rectangle, Subscription, Task, Theme, clipboard, time, window};

/// Widget id of the message list; its visible bounds clip the MCP App webviews.
const MESSAGES_ID: &str = "messages";

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let webviews = app_webview::init();
    iced::application(
        move || ChatApp::boot(webviews),
        ChatApp::update,
        ChatApp::view,
    )
    .subscription(ChatApp::subscription)
    .title("Chat (iced)")
    .theme(Theme::Dark)
    .window_size((1000.0, 700.0))
    .run()
}

#[derive(Debug, Clone)]
enum Message {
    NewChat,
    Select(String),
    Delete(String),
    Compose(text_editor::Action),
    Send,
    Stop,
    Stream(StreamEvent),
    Copy(String),
    DismissError,
    OpenSettings,
    Settings(SettingsMessage),
    WindowReady(Option<ParentWindow>),
    /// Pumps the webviews and re-places them (only while MCP App views exist).
    Tick,
    /// Visible bounds of the MCP App placeholders; the key `""` is the message list.
    AppBounds(Vec<(String, Option<Rectangle>)>),
}

#[derive(Debug, Clone)]
enum SettingsMessage {
    Provider(ProviderKind),
    ApiKey(String),
    RemoveKey,
    Model(String),
    MaxTokens(String),
    SystemPrompt(text_editor::Action),
    Save,
    Cancel,
}

struct SettingsDraft {
    settings: Settings,
    /// Empty means "keep the stored key".
    api_key: String,
    api_key_set: bool,
    max_tokens: String,
    system_prompt: text_editor::Content,
}

struct Streaming {
    conversation_id: String,
    canceller: Canceller,
}

/// MCP App views (child webviews) of the active conversation, keyed by
/// `conversation id:message index` (also the placeholder's widget id).
struct AppViews {
    supported: bool,
    parent: Option<ParentWindow>,
    views: HashMap<String, AppView>,
}

struct AppView {
    webview: AppWebView,
    height: f32,
}

struct ChatApp {
    service: Option<ChatService>,
    apps: AppViews,
    composer: text_editor::Content,
    streaming: Option<Streaming>,
    draft: Option<SettingsDraft>,
    error: Option<String>,
    /// Parsed Markdown of the active conversation, kept in sync in `update` so `view` stays pure.
    blocks: Vec<Vec<Block>>,
}

impl ChatApp {
    fn boot(webviews: bool) -> (Self, Task<Message>) {
        let (service, error) = match ChatService::load() {
            Ok(service) => (Some(service), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let mut app = Self {
            service,
            apps: AppViews {
                supported: webviews,
                parent: None,
                views: HashMap::new(),
            },
            composer: text_editor::Content::new(),
            streaming: None,
            draft: None,
            error,
            blocks: Vec::new(),
        };
        app.reparse();
        let parent = window::oldest()
            .and_then(|id| window::run(id, |w| ParentWindow::of(w)))
            .map(Message::WindowReady);
        (app, parent)
    }

    /// Keys of the MCP App views the active conversation shows.
    fn app_keys(&self) -> Vec<String> {
        let Some(conversation) = self.service.as_ref().and_then(|s| s.workspace.active()) else {
            return Vec::new();
        };
        conversation
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.app.is_some())
            .map(|(i, _)| format!("{}:{i}", conversation.id))
            .collect()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.apps.views.is_empty() && self.app_keys().is_empty() {
            Subscription::none()
        } else {
            // Webview events only arrive while the GTK loop is pumped, so keep ticking.
            time::every(Duration::from_millis(16)).map(|_| Message::Tick)
        }
    }

    /// Pumps the webviews, creates/drops them to match the conversation and
    /// asks for the placeholders' bounds.
    fn tick(&mut self) -> Task<Message> {
        app_webview::pump_platform();
        let mut send = None;
        for view in self.apps.views.values_mut() {
            for event in view.webview.pump() {
                match event {
                    HostEvent::Resize(height) => view.height = height as f32,
                    HostEvent::SendMessage(text) => send = Some(text),
                    HostEvent::OpenLink(url) => mcp_app::open_link(&url),
                }
            }
        }
        let keys = self.app_keys();
        self.apps.views.retain(|key, _| keys.contains(key));
        if let (true, Some(parent), Some(service)) =
            (self.apps.supported, self.apps.parent, &self.service)
        {
            for key in &keys {
                if self.apps.views.contains_key(key) {
                    continue;
                }
                let Some(host) = app_host(service, key) else {
                    continue;
                };
                match AppWebView::new(&parent, host, || {}) {
                    Ok(webview) => {
                        let height = mcp_app::INITIAL_HEIGHT as f32;
                        self.apps
                            .views
                            .insert(key.clone(), AppView { webview, height });
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to create MCP App view: {e}"));
                        self.apps.supported = false;
                    }
                }
            }
        }
        let mut targets: Vec<(String, widget::Id)> = keys
            .into_iter()
            .map(|k| (k.clone(), widget::Id::from(k)))
            .collect();
        targets.push((String::new(), widget::Id::from(MESSAGES_ID)));
        let bounds = selector::find_all(move |candidate: Candidate<'_>| {
            let id = candidate.id()?;
            let (key, _) = targets.iter().find(|(_, target)| target == id)?;
            Some((key.clone(), candidate.visible_bounds()))
        })
        .map(Message::AppBounds);
        match send {
            Some(text) => {
                self.composer = text_editor::Content::with_text(&text);
                Task::batch([bounds, self.update(Message::Send)])
            }
            None => bounds,
        }
    }

    /// Lays the webviews over their placeholders, cut to the message list.
    fn place_apps(&mut self, bounds: &[(String, Option<Rectangle>)]) {
        let visible = |key: &str| bounds.iter().find(|(k, _)| k == key).and_then(|(_, b)| *b);
        let viewport = visible("").filter(|_| self.draft.is_none());
        for (key, view) in &mut self.apps.views {
            let (Some(viewport), Some(shown)) = (viewport, visible(key)) else {
                view.webview.hide();
                continue;
            };
            // Only the visible part is known; rebuild the full placeholder from its height.
            let height = view.height.max(shown.height);
            let y = if shown.height < height && shown.y <= viewport.y + 0.5 {
                shown.y + shown.height - height
            } else {
                shown.y
            };
            let bounds = |r: Rectangle| Bounds {
                x: f64::from(r.x),
                y: f64::from(r.y),
                width: f64::from(r.width),
                height: f64::from(r.height),
            };
            let placeholder = Rectangle { y, height, ..shown };
            view.webview.place(bounds(placeholder), bounds(viewport));
        }
    }

    fn reparse(&mut self) {
        self.blocks = self
            .service
            .as_ref()
            .and_then(|s| s.workspace.active())
            .map(|c| {
                c.messages
                    .iter()
                    .map(|m| chat_core::markdown::parse(&m.content))
                    .collect()
            })
            .unwrap_or_default();
    }

    fn reparse_last(&mut self, id: &str) {
        let Some(conversation) = self.service.as_ref().and_then(|s| s.workspace.active()) else {
            return;
        };
        if conversation.id != id {
            return;
        }
        if let (Some(message), Some(blocks)) =
            (conversation.messages.last(), self.blocks.last_mut())
        {
            *blocks = chat_core::markdown::parse(&message.content);
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let Some(service) = &mut self.service else {
            return Task::none();
        };
        match message {
            Message::NewChat => {
                service.workspace.new_conversation();
                self.draft = None;
                self.reparse();
            }
            Message::Select(id) => {
                service.workspace.select(&id);
                self.draft = None;
                self.reparse();
            }
            Message::Delete(id) => {
                if let Some(s) = self.streaming.as_ref().filter(|s| s.conversation_id == id) {
                    s.canceller.cancel();
                }
                if let Err(e) = service.workspace.delete(&id) {
                    self.error = Some(e.to_string());
                }
                self.reparse();
            }
            Message::Compose(action) => self.composer.perform(action),
            Message::Send => {
                let text = self.composer.text().trim().to_owned();
                if text.is_empty() || self.streaming.is_some() {
                    return Task::none();
                }
                match service.send(&text) {
                    Ok((conversation_id, handle)) => {
                        self.composer = text_editor::Content::new();
                        self.error = None;
                        self.streaming = Some(Streaming {
                            conversation_id,
                            canceller: handle.canceller,
                        });
                        self.reparse();
                        let events = stream::unfold(handle.events, |mut rx| async move {
                            rx.recv().await.map(|event| (event, rx))
                        });
                        return Task::run(events, Message::Stream);
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Message::Stop => {
                if let Some(s) = &self.streaming {
                    s.canceller.cancel();
                }
            }
            Message::Stream(event) => {
                let Some(id) = self.streaming.as_ref().map(|s| s.conversation_id.clone()) else {
                    return Task::none();
                };
                match event {
                    StreamEvent::Delta(delta) => service.workspace.append_delta(&id, &delta),
                    StreamEvent::ToolUse(call) => service.workspace.record_tool_use(&id, &call),
                    StreamEvent::Finished(outcome) => {
                        if let Err(e) = service.workspace.finish_turn(&id, &outcome) {
                            self.error = Some(e.to_string());
                        }
                        self.streaming = None;
                    }
                }
                self.reparse_last(&id);
            }
            Message::Copy(code) => return clipboard::write(code),
            Message::DismissError => self.error = None,
            Message::OpenSettings => {
                let settings = service.settings().clone();
                self.draft = Some(SettingsDraft {
                    max_tokens: settings.max_tokens.to_string(),
                    system_prompt: text_editor::Content::with_text(&settings.system_prompt),
                    settings,
                    api_key: String::new(),
                    api_key_set: ApiKeyStore.is_set(),
                });
            }
            Message::Settings(message) => self.update_settings(message),
            Message::WindowReady(parent) => self.apps.parent = parent,
            Message::Tick => return self.tick(),
            Message::AppBounds(bounds) => self.place_apps(&bounds),
        }
        Task::none()
    }

    fn update_settings(&mut self, message: SettingsMessage) {
        let (Some(draft), Some(service)) = (&mut self.draft, &mut self.service) else {
            return;
        };
        match message {
            SettingsMessage::Provider(kind) => draft.settings.provider = kind,
            SettingsMessage::ApiKey(key) => draft.api_key = key,
            SettingsMessage::RemoveKey => match ApiKeyStore.delete() {
                Ok(()) => draft.api_key_set = false,
                Err(e) => self.error = Some(e.to_string()),
            },
            SettingsMessage::Model(model) => draft.settings.model = model,
            SettingsMessage::MaxTokens(value) => {
                if value.is_empty() || value.parse::<u32>().is_ok() {
                    draft.max_tokens = value;
                }
            }
            SettingsMessage::SystemPrompt(action) => draft.system_prompt.perform(action),
            SettingsMessage::Save => {
                let mut settings = draft.settings.clone();
                settings.max_tokens = draft.max_tokens.parse().unwrap_or(settings.max_tokens);
                settings.system_prompt = draft.system_prompt.text().trim_end().to_owned();
                let key = draft.api_key.trim();
                let result = if key.is_empty() {
                    Ok(())
                } else {
                    ApiKeyStore.set(key)
                };
                match result.and_then(|()| service.save_settings(settings)) {
                    Ok(()) => self.draft = None,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            SettingsMessage::Cancel => self.draft = None,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let Some(service) = &self.service else {
            return container(text(self.error.clone().unwrap_or_default()))
                .padding(20)
                .into();
        };
        let main: Element<'_, Message> = match &self.draft {
            Some(draft) => settings_view(draft),
            None => column![self.messages(service), rule::horizontal(1), self.composer()].into(),
        };
        let main = match &self.error {
            Some(error) => column![
                container(
                    row![
                        text(error.clone()).style(text::danger).width(Fill),
                        button("✕")
                            .style(button::text)
                            .on_press(Message::DismissError)
                    ]
                    .spacing(8)
                )
                .padding(8),
                main
            ]
            .into(),
            None => main,
        };
        row![sidebar(service), rule::vertical(1), main].into()
    }

    fn messages<'a>(&'a self, service: &'a ChatService) -> Element<'a, Message> {
        let Some(conversation) = service.workspace.active() else {
            return container(text("Start a new conversation"))
                .center(Fill)
                .into();
        };
        let items = conversation
            .messages
            .iter()
            .zip(&self.blocks)
            .enumerate()
            .map(|(index, (message, blocks))| {
                let (name, body): (&str, Element<'_, Message>) = match message.role {
                    Role::User => ("You", text(&message.content).into()),
                    Role::Assistant => ("Assistant", markdown::view(blocks)),
                };
                let mut bubble = column![text(name).font(markdown::BOLD), body].spacing(6);
                if message.app.is_some() {
                    bubble =
                        bubble.push(self.app_placeholder(format!("{}:{index}", conversation.id)));
                }
                if let Some(error) = &message.error {
                    bubble = bubble.push(text(format!("⚠ {error}")).style(text::danger));
                }
                container(bubble)
                    .padding(10)
                    .width(Fill)
                    .style(container::rounded_box)
                    .into()
            });
        scrollable(column(items).spacing(8).padding(12).max_width(820))
            .id(MESSAGES_ID)
            .anchor_bottom()
            .height(Fill)
            .width(Fill)
            .into()
    }

    /// Reserves the space of an MCP App view; the webview is laid over it in `place_apps`.
    fn app_placeholder(&self, key: String) -> Element<'_, Message> {
        if !self.apps.supported {
            return text("(MCP App views need X11 on Linux)").into();
        }
        let height = self
            .apps
            .views
            .get(&key)
            .map_or(mcp_app::INITIAL_HEIGHT as f32, |v| v.height);
        container(space::horizontal())
            .id(key)
            .width(Fill)
            .height(height)
            .style(container::bordered_box)
            .into()
    }

    fn composer(&self) -> Element<'_, Message> {
        let editor = text_editor(&self.composer)
            .placeholder("Message… (Enter to send, Shift+Enter for newline)")
            .height(80)
            .on_action(Message::Compose)
            .key_binding(|press| match press.key {
                Key::Named(Named::Enter) if !press.modifiers.shift() => {
                    Some(text_editor::Binding::Custom(Message::Send))
                }
                _ => text_editor::Binding::from_key_press(press),
            });
        let action = if self.streaming.is_some() {
            button("⏹ Stop")
                .style(button::danger)
                .on_press(Message::Stop)
        } else {
            button("Send").on_press(Message::Send)
        };
        row![editor, action.width(80)].spacing(8).padding(10).into()
    }
}

fn app_host(service: &ChatService, key: &str) -> Option<AppHost> {
    let (id, index) = key.rsplit_once(':')?;
    let message = service
        .workspace
        .get(id)?
        .messages
        .get(index.parse::<usize>().ok()?)?;
    AppHost::new(message.app.clone()?, Some(mcp_app::Theme::Dark))
}

fn sidebar(service: &ChatService) -> Element<'_, Message> {
    let active = service.workspace.active_id();
    let items = service.workspace.conversations().iter().map(|c| {
        let style = if active == Some(c.id.as_str()) {
            button::primary
        } else {
            button::text
        };
        row![
            button(text(&c.title).wrapping(text::Wrapping::None))
                .style(style)
                .width(Fill)
                .on_press(Message::Select(c.id.clone())),
            button("🗑")
                .style(button::text)
                .on_press(Message::Delete(c.id.clone())),
        ]
        .into()
    });
    column![
        row![
            button("+ New chat").on_press(Message::NewChat),
            button("Settings")
                .style(button::secondary)
                .on_press(Message::OpenSettings),
        ]
        .spacing(8),
        rule::horizontal(1),
        scrollable(column(items).spacing(2)).height(Fill),
    ]
    .spacing(8)
    .padding(8)
    .width(Length::Fixed(240.0))
    .into()
}

fn settings_view(draft: &SettingsDraft) -> Element<'_, Message> {
    let field =
        |label: &'static str, input: Element<'static, Message>| -> Element<'static, Message> {
            row![text(label).width(120), input].spacing(12).into()
        };
    let msg = |f: fn(String) -> SettingsMessage| move |value: String| Message::Settings(f(value));

    let mut api_key = column![
        text_input(
            if draft.api_key_set {
                "•••• (saved in keychain)"
            } else {
                "sk-ant-…"
            },
            &draft.api_key
        )
        .secure(true)
        .on_input(msg(SettingsMessage::ApiKey))
    ]
    .spacing(4);
    if draft.api_key_set {
        api_key = api_key.push(
            button(text("Remove saved key").size(12))
                .style(button::text)
                .on_press(Message::Settings(SettingsMessage::RemoveKey)),
        );
    }

    let view = column![
        text("Settings").size(24),
        field(
            "Provider",
            pick_list(ProviderKind::ALL, Some(draft.settings.provider), |k| {
                Message::Settings(SettingsMessage::Provider(k))
            })
            .into()
        ),
        row![text("API key").width(120), api_key].spacing(12),
        row![
            text("Model").width(120),
            text_input("model", &draft.settings.model).on_input(msg(SettingsMessage::Model))
        ]
        .spacing(12),
        row![
            text("Max tokens").width(120),
            text_input("4096", &draft.max_tokens).on_input(msg(SettingsMessage::MaxTokens))
        ]
        .spacing(12),
        row![
            text("System prompt").width(120),
            text_editor(&draft.system_prompt)
                .height(140)
                .on_action(|a| Message::Settings(SettingsMessage::SystemPrompt(a)))
        ]
        .spacing(12),
        row![
            button("Save").on_press(Message::Settings(SettingsMessage::Save)),
            button("Cancel")
                .style(button::secondary)
                .on_press(Message::Settings(SettingsMessage::Cancel)),
            space::horizontal(),
        ]
        .spacing(8),
    ]
    .spacing(12)
    .padding(20)
    .max_width(640);
    scrollable(view).height(Fill).width(Fill).into()
}
