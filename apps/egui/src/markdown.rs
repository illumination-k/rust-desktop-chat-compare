//! Renders `chat_core::markdown` blocks with egui widgets.

use chat_core::Conversation;
use chat_core::markdown::{self, Block, Span};
use eframe::egui::{self, text::LayoutJob};

/// Parsed blocks per message of one conversation. Messages only ever grow
/// (by appended tokens), so the content length is a sufficient cache key.
#[derive(Default)]
pub struct Cache {
    conversation_id: String,
    entries: Vec<(usize, Vec<Block>)>,
}

impl Cache {
    pub fn get(&mut self, conversation: &Conversation) -> &[(usize, Vec<Block>)] {
        if self.conversation_id != conversation.id {
            self.conversation_id.clone_from(&conversation.id);
            self.entries.clear();
        }
        self.entries.truncate(conversation.messages.len());
        for (i, message) in conversation.messages.iter().enumerate() {
            let len = message.content.len();
            match self.entries.get_mut(i) {
                Some(entry) if entry.0 == len => {}
                Some(entry) => *entry = (len, markdown::parse(&message.content)),
                None => self.entries.push((len, markdown::parse(&message.content))),
            }
        }
        &self.entries
    }
}

pub fn show(ui: &mut egui::Ui, (_, blocks): &(usize, Vec<Block>)) {
    for block in blocks {
        match block {
            Block::Heading { level, spans } => {
                let size = match level {
                    1 => 22.0,
                    2 => 19.0,
                    _ => 16.0,
                };
                ui.label(job(ui, spans, Some(size)));
            }
            Block::Paragraph(spans) => {
                ui.label(job(ui, spans, None));
            }
            Block::ListItem {
                depth,
                number,
                spans,
            } => {
                ui.horizontal_wrapped(|ui| {
                    #[allow(clippy::cast_precision_loss)]
                    ui.add_space(12.0 + 16.0 * *depth as f32);
                    let marker = number.map_or_else(|| "•".to_owned(), |n| format!("{n}."));
                    ui.label(marker);
                    ui.label(job(ui, spans, None));
                });
            }
            Block::Quote(spans) => {
                ui.horizontal(|ui| {
                    ui.separator();
                    ui.label(job(ui, spans, None));
                });
            }
            Block::CodeBlock { lang, code } => {
                egui::Frame::new()
                    .fill(ui.visuals().code_bg_color)
                    .inner_margin(8.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.weak(lang);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Copy").clicked() {
                                        ui.ctx().copy_text(code.clone());
                                    }
                                },
                            );
                        });
                        ui.add(egui::Label::new(egui::RichText::new(code).monospace()).wrap());
                    });
            }
            Block::Rule => {
                ui.separator();
            }
        }
    }
}

fn job(ui: &egui::Ui, spans: &[Span], size: Option<f32>) -> LayoutJob {
    let style = ui.style();
    let body = size.map_or_else(
        || egui::TextStyle::Body.resolve(style),
        egui::FontId::proportional,
    );
    let mut job = LayoutJob::default();
    for span in spans {
        let mut format = egui::TextFormat {
            font_id: if span.code {
                egui::FontId::monospace(body.size)
            } else {
                body.clone()
            },
            color: if span.bold || size.is_some() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().text_color()
            },
            italics: span.italic,
            ..Default::default()
        };
        if span.code {
            format.background = ui.visuals().code_bg_color;
        }
        job.append(&span.text, 0.0, format);
    }
    job
}
