//! Markdown rendering helpers.
//!
//! Native UIs use [`parse`] to get a flat list of [`Block`]s; web UIs use [`to_html`].
//! Only the common subset (headings, paragraphs, lists, quotes, code) is modelled.

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    /// One list item; nested lists are flattened using `depth` (0 = top level).
    ListItem {
        depth: usize,
        /// `Some(n)` for ordered lists, `None` for bullets.
        number: Option<u64>,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    CodeBlock {
        lang: String,
        code: String,
    },
    Rule,
}

impl Block {
    /// Plain text of the block's spans (empty for code blocks and rules).
    pub fn plain_text(&self) -> String {
        match self {
            Self::Heading { spans, .. }
            | Self::Paragraph(spans)
            | Self::ListItem { spans, .. }
            | Self::Quote(spans) => spans.iter().map(|s| s.text.as_str()).collect(),
            Self::CodeBlock { .. } | Self::Rule => String::new(),
        }
    }
}

#[derive(Default)]
struct BlockBuilder {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    bold: usize,
    italic: usize,
    quote: usize,
    heading: Option<u8>,
    /// Next number per open list (`None` = bullet list).
    lists: Vec<Option<u64>>,
    code: Option<(String, String)>,
}

impl BlockBuilder {
    fn text(&mut self, text: &str, code: bool) {
        if let Some((_, buf)) = &mut self.code {
            buf.push_str(text);
            return;
        }
        let (bold, italic) = (self.bold > 0, self.italic > 0);
        match self.spans.last_mut() {
            Some(last) if !code && !last.code && last.bold == bold && last.italic == italic => {
                last.text.push_str(text)
            }
            _ => self.spans.push(Span {
                text: text.to_owned(),
                bold,
                italic,
                code,
            }),
        }
    }

    /// Emits the pending spans as the block implied by the current context.
    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let block = if let Some(level) = self.heading.take() {
            Block::Heading { level, spans }
        } else if let Some(next) = self.lists.last_mut() {
            let number = *next;
            if let Some(n) = next {
                *n += 1;
            }
            Block::ListItem {
                depth: self.lists.len() - 1,
                number,
                spans,
            }
        } else if self.quote > 0 {
            Block::Quote(spans)
        } else {
            Block::Paragraph(spans)
        };
        self.blocks.push(block);
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    self.flush();
                    self.heading = Some(level as u8);
                }
                Tag::List(start) => {
                    self.flush();
                    self.lists.push(start);
                }
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.quote += 1;
                }
                Tag::CodeBlock(kind) => {
                    self.flush();
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_owned(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::Strong => self.bold += 1,
                Tag::Emphasis => self.italic += 1,
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) | TagEnd::Item => self.flush(),
                TagEnd::Paragraph if self.lists.is_empty() => self.flush(),
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quote = self.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, mut code)) = self.code.take() {
                        if code.ends_with('\n') {
                            code.pop();
                        }
                        self.blocks.push(Block::CodeBlock { lang, code });
                    }
                }
                TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
                TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                self.text(&text, false)
            }
            Event::Code(text) => self.text(&text, true),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.text("\n", false),
            Event::Rule => {
                self.flush();
                self.blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
}

/// Parses Markdown into flat blocks. An unterminated code fence (mid-stream) is still emitted.
pub fn parse(markdown: &str) -> Vec<Block> {
    let mut builder = BlockBuilder::default();
    for event in Parser::new_ext(markdown, Options::empty()) {
        builder.event(event);
    }
    builder.flush();
    builder.blocks
}

/// Renders Markdown to HTML that is safe to assign to `innerHTML`:
/// raw HTML is escaped and only http(s)/mailto links are kept.
pub fn to_html(markdown: &str) -> String {
    let events = Parser::new_ext(markdown, Options::empty()).map(|event| match event {
        Event::Html(html) | Event::InlineHtml(html) => Event::Text(html),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        })
        | Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
        other => other,
    });
    let mut html = String::with_capacity(markdown.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

fn safe_url(url: CowStr<'_>) -> CowStr<'_> {
    let lower = url.trim_start().to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        url
    } else {
        CowStr::Borrowed("#")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str) -> Span {
        Span {
            text: text.into(),
            ..Span::default()
        }
    }

    #[test]
    fn parses_common_blocks() {
        let md = "# Title\n\nSome **bold** and `code`.\n\n- a\n- b\n  1. x\n  2. y\n\n```rust\nfn main() {}\n```\n\n> quote\n\n---\n";
        let blocks = parse(md);
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 1,
                    spans: vec![span("Title")]
                },
                Block::Paragraph(vec![
                    span("Some "),
                    Span {
                        text: "bold".into(),
                        bold: true,
                        ..Span::default()
                    },
                    span(" and "),
                    Span {
                        text: "code".into(),
                        code: true,
                        ..Span::default()
                    },
                    span("."),
                ]),
                Block::ListItem {
                    depth: 0,
                    number: None,
                    spans: vec![span("a")]
                },
                Block::ListItem {
                    depth: 0,
                    number: None,
                    spans: vec![span("b")]
                },
                Block::ListItem {
                    depth: 1,
                    number: Some(1),
                    spans: vec![span("x")]
                },
                Block::ListItem {
                    depth: 1,
                    number: Some(2),
                    spans: vec![span("y")]
                },
                Block::CodeBlock {
                    lang: "rust".into(),
                    code: "fn main() {}".into()
                },
                Block::Quote(vec![span("quote")]),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn unterminated_code_fence_is_emitted() {
        assert_eq!(
            parse("```\nlet x"),
            vec![Block::CodeBlock {
                lang: String::new(),
                code: "let x".into()
            }]
        );
    }

    #[test]
    fn loose_list_items_are_single_blocks() {
        let blocks = parse("1. one\n\n2. two\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].plain_text(), "two");
    }

    #[test]
    fn html_output_escapes_raw_html_and_unsafe_links() {
        let html = to_html(
            "<script>alert(1)</script>\n\n[x](javascript:alert(1)) [y](https://example.com)",
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(r##"<a href="#">x</a>"##));
        assert!(html.contains(r#"<a href="https://example.com">y</a>"#));
    }

    #[test]
    fn html_renders_code_blocks() {
        assert!(to_html("```rust\nfn f() {}\n```").contains(r#"<code class="language-rust">"#));
    }
}
