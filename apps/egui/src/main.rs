//! AI chat app built with egui (eframe).

mod markdown;

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use app_webview::{AppWebView, Bounds};
use chat_core::mcp_app::{self, AppHost, HostEvent, Theme};
use chat_core::stream::Canceller;
use chat_core::{ApiKeyStore, ChatService, ProviderKind, Role, Settings, StreamEvent};
use eframe::egui;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    // eframe owns the main thread; streams run on this runtime.
    let runtime =
        tokio::runtime::Runtime::new().map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let _guard = runtime.enter();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Chat (egui)")
            .with_inner_size([1000.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "chat-egui",
        options,
        Box::new(|cc| {
            install_cjk_font(&cc.egui_ctx);
            let mut app = ChatApp::new(ChatService::load()?);
            app.apps.supported = app_webview::init();
            Ok(Box::new(app))
        }),
    )
}

/// egui only ships Latin fonts, so borrow a CJK font from the OS for Japanese text.
fn install_cjk_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    ];
    let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) else {
        tracing::warn!("no CJK font found; Japanese text will render as tofu");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}

#[derive(PartialEq, Eq)]
enum View {
    Chat,
    Settings,
}

struct Streaming {
    conversation_id: String,
    canceller: Canceller,
    events: mpsc::Receiver<StreamEvent>,
}

struct SettingsDraft {
    settings: Settings,
    /// Empty means "keep the stored key".
    api_key: String,
    api_key_set: bool,
}

/// MCP App views (child webviews) keyed by `conversation id:message index`.
#[derive(Default)]
struct AppViews {
    supported: bool,
    views: HashMap<String, AppView>,
    /// Placeholders drawn this frame: key, placeholder and visible area (egui points).
    placed: Vec<(String, egui::Rect, egui::Rect)>,
}

struct AppView {
    webview: AppWebView,
    height: f32,
}

struct ChatApp {
    service: ChatService,
    apps: AppViews,
    view: View,
    input: String,
    streaming: Option<Streaming>,
    draft: Option<SettingsDraft>,
    error: Option<String>,
    markdown: markdown::Cache,
}

impl ChatApp {
    fn new(service: ChatService) -> Self {
        Self {
            service,
            apps: AppViews::default(),
            view: View::Chat,
            input: String::new(),
            streaming: None,
            draft: None,
            error: None,
            markdown: markdown::Cache::default(),
        }
    }

    fn send(&mut self, ctx: &egui::Context) {
        let text = self.input.trim().to_owned();
        if text.is_empty() || self.streaming.is_some() {
            return;
        }
        match self.service.send(&text) {
            Ok((conversation_id, mut handle)) => {
                self.input.clear();
                self.error = None;
                // Forward tokens to the UI thread and wake egui for each one.
                let (tx, rx) = mpsc::channel();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    while let Some(event) = handle.events.recv().await {
                        if tx.send(event).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                });
                self.streaming = Some(Streaming {
                    conversation_id,
                    canceller: handle.canceller,
                    events: rx,
                });
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn drain_stream(&mut self) {
        let Some(streaming) = &self.streaming else {
            return;
        };
        let id = streaming.conversation_id.clone();
        while let Ok(event) = streaming.events.try_recv() {
            match event {
                StreamEvent::Delta(text) => self.service.workspace.append_delta(&id, &text),
                StreamEvent::ToolUse(call) => self.service.workspace.record_tool_use(&id, &call),
                StreamEvent::Finished(outcome) => {
                    if let Err(e) = self.service.workspace.finish_turn(&id, &outcome) {
                        self.error = Some(e.to_string());
                    }
                    self.streaming = None;
                    return;
                }
            }
        }
    }

    /// Handles messages from the views and what they ask the host to do.
    fn pump_apps(&mut self, ctx: &egui::Context) {
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
        if let Some(text) = send {
            self.input = text;
            self.send(ctx);
        }
        if !self.apps.views.is_empty() {
            // Webview events only arrive while the GTK loop is pumped, so keep ticking.
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// Creates, moves, hides or drops webviews to match the placeholders drawn this frame.
    fn place_apps(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let placed = std::mem::take(&mut self.apps.placed);
        let active = self
            .service
            .workspace
            .active_id()
            .unwrap_or_default()
            .to_owned();
        let prefix = format!("{active}:");
        self.apps.views.retain(|key, _| key.starts_with(&prefix));
        let zoom = f64::from(ctx.zoom_factor());
        let bounds = |r: egui::Rect| Bounds {
            x: f64::from(r.min.x) * zoom,
            y: f64::from(r.min.y) * zoom,
            width: f64::from(r.width()) * zoom,
            height: f64::from(r.height()) * zoom,
        };
        for (key, view) in &mut self.apps.views {
            if !placed.iter().any(|(k, ..)| k == key) {
                view.webview.hide();
            }
        }
        for (key, rect, clip) in placed {
            if !self.apps.views.contains_key(&key) {
                let Some(host) = self.app_host(&key, ctx) else {
                    continue;
                };
                let wake = ctx.clone();
                match AppWebView::new(frame, host, move || wake.request_repaint()) {
                    Ok(webview) => {
                        let height = mcp_app::INITIAL_HEIGHT as f32;
                        self.apps
                            .views
                            .insert(key.clone(), AppView { webview, height });
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to create MCP App view: {e}"));
                        self.apps.supported = false;
                        continue;
                    }
                }
            }
            if let Some(view) = self.apps.views.get_mut(&key) {
                view.webview.place(bounds(rect), bounds(clip));
            }
        }
    }

    fn app_host(&self, key: &str, ctx: &egui::Context) -> Option<AppHost> {
        let (id, index) = key.rsplit_once(':')?;
        let message = self
            .service
            .workspace
            .get(id)?
            .messages
            .get(index.parse::<usize>().ok()?)?;
        let theme = if ctx.theme() == egui::Theme::Dark {
            Theme::Dark
        } else {
            Theme::Light
        };
        AppHost::new(message.app.clone()?, Some(theme))
    }

    fn open_settings(&mut self) {
        self.draft = Some(SettingsDraft {
            settings: self.service.settings().clone(),
            api_key: String::new(),
            api_key_set: ApiKeyStore.is_set(),
        });
        self.view = View::Settings;
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("➕ New chat").clicked() {
                self.service.workspace.new_conversation();
                self.view = View::Chat;
            }
            if ui.button("⚙ Settings").clicked() {
                self.open_settings();
            }
        });
        ui.separator();
        let active = self.service.workspace.active_id().map(str::to_owned);
        let mut select = None;
        let mut delete = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for conversation in self.service.workspace.conversations() {
                ui.horizontal(|ui| {
                    let is_active = active.as_deref() == Some(conversation.id.as_str());
                    if ui
                        .selectable_label(is_active, &conversation.title)
                        .clicked()
                    {
                        select = Some(conversation.id.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("Delete").clicked() {
                            delete = Some(conversation.id.clone());
                        }
                    });
                });
            }
        });
        if let Some(id) = select {
            self.service.workspace.select(&id);
            self.view = View::Chat;
        }
        if let Some(id) = delete {
            if let Some(streaming) = self.streaming.as_ref().filter(|s| s.conversation_id == id) {
                streaming.canceller.cancel();
            }
            if let Err(e) = self.service.workspace.delete(&id) {
                self.error = Some(e.to_string());
            }
        }
    }

    fn composer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let streaming = self.streaming.is_some();
            let button_width = 80.0;
            let id = egui::Id::new("composer");
            // Consume plain Enter before the editor sees it, so it sends instead of
            // inserting a newline (Shift+Enter still reaches the editor).
            let enter = ui.memory(|m| m.has_focus(id))
                // `consume_key` ignores Shift, so check the modifiers explicitly.
                && ui.input_mut(|i| {
                    i.modifiers.is_none() && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                });
            let editor = egui::TextEdit::multiline(&mut self.input)
                .id(id)
                .hint_text("Message… (Enter to send, Shift+Enter for newline)")
                .desired_rows(3)
                .desired_width(ui.available_width() - button_width - 8.0);
            let response = ui.add(editor);
            if streaming {
                if ui
                    .add_sized([button_width, 32.0], egui::Button::new("⏹ Stop"))
                    .clicked()
                    && let Some(s) = &self.streaming
                {
                    s.canceller.cancel();
                }
            } else if ui
                .add_sized([button_width, 32.0], egui::Button::new("Send"))
                .clicked()
                || enter
            {
                self.send(ui.ctx());
                response.request_focus();
            }
        });
        ui.add_space(8.0);
    }

    fn messages(&mut self, ui: &mut egui::Ui) {
        let Some(conversation) = self.service.workspace.active() else {
            ui.centered_and_justified(|ui| ui.label("Start a new conversation"));
            return;
        };
        let blocks = self.markdown.get(conversation);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(820.0));
                for (index, (message, blocks)) in
                    conversation.messages.iter().zip(blocks).enumerate()
                {
                    let (name, fill) = match message.role {
                        Role::User => ("You", ui.visuals().faint_bg_color),
                        Role::Assistant => ("Assistant", ui.visuals().extreme_bg_color),
                    };
                    egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.strong(name);
                        match message.role {
                            Role::User => {
                                ui.label(&message.content);
                            }
                            Role::Assistant => markdown::show(ui, blocks),
                        }
                        if message.app.is_some() {
                            let key = format!("{}:{index}", conversation.id);
                            app_placeholder(ui, &mut self.apps, key);
                        }
                        if let Some(error) = &message.error {
                            ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {error}"));
                        }
                    });
                    ui.add_space(6.0);
                }
            });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        ui.heading("Settings");
        ui.add_space(8.0);
        egui::Grid::new("settings")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Provider");
                ui.horizontal(|ui| {
                    for kind in ProviderKind::ALL {
                        ui.radio_value(&mut draft.settings.provider, kind, kind.label());
                    }
                });
                ui.end_row();

                ui.label("API key");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.api_key)
                            .password(true)
                            .hint_text(if draft.api_key_set {
                                "•••• (saved in keychain)"
                            } else {
                                "sk-ant-…"
                            })
                            .desired_width(360.0),
                    );
                    if draft.api_key_set && ui.small_button("Remove saved key").clicked() {
                        match ApiKeyStore.delete() {
                            Ok(()) => draft.api_key_set = false,
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
                ui.end_row();

                ui.label("Model");
                ui.add(egui::TextEdit::singleline(&mut draft.settings.model).desired_width(360.0));
                ui.end_row();

                ui.label("Max tokens");
                ui.add(egui::DragValue::new(&mut draft.settings.max_tokens).range(1..=64_000));
                ui.end_row();

                ui.label("System prompt");
                ui.add(
                    egui::TextEdit::multiline(&mut draft.settings.system_prompt)
                        .desired_rows(6)
                        .desired_width(360.0),
                );
                ui.end_row();
            });
        ui.add_space(12.0);
        let (save, cancel) = ui
            .horizontal(|ui| (ui.button("Save").clicked(), ui.button("Cancel").clicked()))
            .inner;
        if save {
            let key = draft.api_key.trim();
            let result = if key.is_empty() {
                Ok(())
            } else {
                ApiKeyStore.set(key)
            };
            let settings = draft.settings.clone();
            match result.and_then(|()| self.service.save_settings(settings)) {
                Ok(()) => self.close_settings(),
                Err(e) => self.error = Some(e.to_string()),
            }
        } else if cancel {
            self.close_settings();
        }
    }

    fn close_settings(&mut self) {
        self.draft = None;
        self.view = View::Chat;
    }
}

/// Reserves the space of an MCP App view; the webview is laid over it in `place_apps`.
fn app_placeholder(ui: &mut egui::Ui, apps: &mut AppViews, key: String) {
    if !apps.supported {
        ui.weak("(MCP App views need X11 on Linux)");
        return;
    }
    let height = apps
        .views
        .get(&key)
        .map_or(mcp_app::INITIAL_HEIGHT as f32, |v| v.height);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    ui.painter().rect_stroke(
        rect,
        6.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    apps.placed.push((key, rect.shrink(1.0), ui.clip_rect()));
}

impl eframe::App for ChatApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_stream();
        self.pump_apps(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| self.sidebar(ui));
        if let Some(error) = self.error.clone() {
            egui::Panel::top("error").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.small_button("✖").clicked() {
                        self.error = None;
                    }
                });
            });
        }
        if self.view == View::Chat {
            egui::Panel::bottom("composer").show(ui, |ui| self.composer(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| match self.view {
            View::Chat => self.messages(ui),
            View::Settings => self.settings_view(ui),
        });
        self.place_apps(&ui.ctx().clone(), frame);
    }
}
