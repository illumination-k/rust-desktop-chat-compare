//! Renders `chat_core::markdown` blocks with iced widgets.

use chat_core::markdown::{Block, Span};
use iced::widget::{button, column, container, rich_text, row, rule, span, text};
use iced::{Element, Fill, Font, font};

use crate::Message;

pub const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};

pub fn view(blocks: &[Block]) -> Element<'_, Message> {
    column(blocks.iter().map(block)).spacing(6).into()
}

fn block(block: &Block) -> Element<'_, Message> {
    match block {
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 24,
                2 => 20,
                _ => 17,
            };
            rich(spans).size(size).font(BOLD).into()
        }
        Block::Paragraph(spans) => rich(spans).into(),
        Block::ListItem {
            depth,
            number,
            spans,
        } => {
            let marker = number.map_or_else(|| "•".to_owned(), |n| format!("{n}."));
            #[allow(clippy::cast_precision_loss)]
            let indent = 12.0 + 16.0 * *depth as f32;
            row![text(marker), rich(spans)]
                .spacing(6)
                .padding(iced::Padding::ZERO.left(indent))
                .into()
        }
        Block::Quote(spans) => row![rule::vertical(2), rich(spans)]
            .spacing(8)
            .height(iced::Shrink)
            .into(),
        Block::CodeBlock { lang, code } => container(
            column![
                row![
                    text(lang).size(12).width(Fill),
                    button(text("Copy").size(12))
                        .style(button::text)
                        .on_press(Message::Copy(code.clone())),
                ],
                text(code).font(Font::MONOSPACE),
            ]
            .spacing(4),
        )
        .padding(8)
        .width(Fill)
        .style(container::dark)
        .into(),
        Block::Rule => rule::horizontal(1).into(),
    }
}

fn rich(spans: &[Span]) -> iced::widget::text::Rich<'_, (), Message> {
    rich_text(
        spans
            .iter()
            .map(|s| {
                let font = Font {
                    family: if s.code {
                        font::Family::Monospace
                    } else {
                        font::Family::SansSerif
                    },
                    weight: if s.bold {
                        font::Weight::Bold
                    } else {
                        font::Weight::Normal
                    },
                    style: if s.italic {
                        font::Style::Italic
                    } else {
                        font::Style::Normal
                    },
                    ..Font::DEFAULT
                };
                span(s.text.as_str()).font(font)
            })
            .collect::<Vec<_>>(),
    )
}
