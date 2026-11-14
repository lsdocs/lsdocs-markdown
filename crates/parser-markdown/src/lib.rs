//! First-party Markdown parser plugin.
//!
//! Parses CommonMark with GitHub-style tables and task lists (via
//! `pulldown-cmark`) into a document tree. Every node carries the byte span
//! it came from.
//!
//! - Front matter: YAML between `---` fences, or TOML between `+++` fences,
//!   at the very start of the file. It must be a mapping; it becomes the
//!   document's metadata.
//! - Headings get an `id`: an explicit `{#id}` attribute, or a slug of their
//!   text, made unique within the document (`intro`, `intro-1`, ...).
//! - Raw HTML becomes `raw` nodes with format `html`.
//! - Soft line breaks become `\n` within text; hard breaks become
//!   `line_break` nodes.

use std::collections::HashMap;
use std::ops::Range;

use lsdocs_plugin_sdk::doctree::{
    Align, BlockQuote, CodeBlock, Document, Emphasis, Heading, Image, InlineCode, LineBreak, Link,
    List, ListItem, Metadata, Node, Paragraph, Raw, Span, Strong, Table, TableCell, TableRow, Text,
    ThematicBreak, Value, plain_text,
};
use lsdocs_plugin_sdk::protocol::{Diagnostic, ParseParams, ParseResult, Severity};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};
use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, MetadataBlockKind, Options, Parser as Cmark, Tag, TagEnd,
};

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-markdown", env!("CARGO_PKG_VERSION")).parser(Markdown)
}

/// The Markdown parser.
#[derive(Debug, Clone, Copy, Default)]
pub struct Markdown;

impl Parser for Markdown {
    fn extensions(&self) -> Vec<String> {
        vec!["md".into(), "markdown".into()]
    }

    async fn parse(&self, params: ParseParams, _ctx: &Context) -> PluginResult<ParseResult> {
        let (document, diagnostics) = parse(&params.source, &params.text);
        Ok(ParseResult {
            document,
            diagnostics,
        })
    }
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

/// Parses Markdown into a document, plus any problems found (e.g. invalid
/// front matter).
pub fn parse(source: &str, input: &str) -> (Document, Vec<Diagnostic>) {
    let mut builder = Builder::new(source);
    for (event, range) in Cmark::new_ext(input, options()).into_offset_iter() {
        builder.event(event, range);
    }
    builder.finish()
}

/// What an open element is collecting.
enum Frame {
    /// A node whose children are collected into it.
    Node(Node),
    /// A code block; text is its value.
    Code(CodeBlock),
    /// An image; text inside is its alt text.
    Image(Image),
    /// Raw HTML; text is its value.
    Html(Raw),
    /// Front matter; text is its source.
    Metadata(MetadataBlockKind, Span, String),
    /// A table head: cells collected into a header row.
    TableHead(TableRow),
}

struct Builder {
    document: Document,
    stack: Vec<Frame>,
    diagnostics: Vec<Diagnostic>,
    /// Column alignments of the table being built.
    aligns: Vec<Align>,
}

fn span(range: &Range<usize>) -> Option<Span> {
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Some(Span::new(clamp(range.start), clamp(range.end)))
}

impl Builder {
    fn new(source: &str) -> Self {
        Self {
            document: Document::new(source),
            stack: Vec::new(),
            diagnostics: Vec::new(),
            aligns: Vec::new(),
        }
    }

    /// Adds a finished node to the innermost open element.
    fn push(&mut self, node: Node) {
        let children = match self.stack.last_mut() {
            None => &mut self.document.children,
            Some(Frame::Node(parent)) => match parent.children_mut() {
                Some(children) => children,
                None => return,
            },
            Some(Frame::TableHead(row)) => &mut row.children,
            // Leaf frames take text, not nodes; anything else is dropped.
            Some(_) => return,
        };
        // Merge adjacent text so soft breaks don't fragment paragraphs.
        if let (Node::Text(new), Some(Node::Text(prev))) = (&node, children.last_mut()) {
            prev.value.push_str(&new.value);
            if let (Some(a), Some(b)) = (prev.span, new.span) {
                prev.span = Some(Span::new(a.start.min(b.start), a.end.max(b.end)));
            }
            return;
        }
        children.push(node);
    }

    /// The alt text of the innermost open image, if inside one. Everything
    /// inside an image (even nested emphasis) only contributes alt text.
    fn image_alt(&mut self) -> Option<&mut String> {
        self.stack.iter_mut().rev().find_map(|frame| match frame {
            Frame::Image(image) => Some(&mut image.alt),
            _ => None,
        })
    }

    fn text(&mut self, value: &str, range: &Range<usize>) {
        if let Some(alt) = self.image_alt() {
            alt.push_str(value);
            return;
        }
        match self.stack.last_mut() {
            Some(Frame::Code(code)) => code.value.push_str(value),
            Some(Frame::Html(raw)) => raw.value.push_str(value),
            Some(Frame::Metadata(_, _, buf)) => buf.push_str(value),
            _ => self.push(Node::Text(Text {
                span: span(range),
                value: value.to_owned(),
            })),
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag, &range),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t, &range),
            Event::Code(code) => match self.image_alt() {
                Some(alt) => alt.push_str(&code),
                None => self.push(Node::InlineCode(InlineCode {
                    span: span(&range),
                    value: code.into_string(),
                })),
            },
            Event::Html(html) | Event::InlineHtml(html) => match self.stack.last_mut() {
                Some(Frame::Html(raw)) => raw.value.push_str(&html),
                _ => self.push(Node::Raw(Raw {
                    span: span(&range),
                    format: "html".into(),
                    value: html.into_string(),
                })),
            },
            Event::SoftBreak => self.text("\n", &range),
            Event::HardBreak => self.push(Node::LineBreak(LineBreak { span: span(&range) })),
            Event::Rule => self.push(Node::ThematicBreak(ThematicBreak { span: span(&range) })),
            Event::TaskListMarker(checked) => {
                if let Some(Frame::Node(Node::ListItem(item))) = self.stack.last_mut() {
                    item.checked = Some(checked);
                }
            }
            // Not enabled: math and footnotes.
            Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: &Range<usize>) {
        let span = span(range);
        let frame = match tag {
            Tag::Paragraph => Frame::Node(Node::Paragraph(Paragraph {
                span,
                children: vec![],
            })),
            Tag::Heading { level, id, .. } => Frame::Node(Node::Heading(Heading {
                span,
                level: level as u8,
                id: id.map(|id| id.into_string()),
                children: vec![],
            })),
            Tag::BlockQuote(_) => Frame::Node(Node::BlockQuote(BlockQuote {
                span,
                children: vec![],
            })),
            Tag::CodeBlock(kind) => {
                let (language, info) = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let info = info.trim();
                        match info.split_once(char::is_whitespace) {
                            Some((lang, rest)) => {
                                (Some(lang.to_owned()), Some(rest.trim().to_owned()))
                            }
                            None if info.is_empty() => (None, None),
                            None => (Some(info.to_owned()), None),
                        }
                    }
                    CodeBlockKind::Indented => (None, None),
                };
                Frame::Code(CodeBlock {
                    span,
                    language,
                    info,
                    value: String::new(),
                })
            }
            Tag::HtmlBlock => Frame::Html(Raw {
                span,
                format: "html".into(),
                value: String::new(),
            }),
            Tag::List(start) => Frame::Node(Node::List(List {
                span,
                ordered: start.is_some(),
                start: start.filter(|&n| n != 1),
                children: vec![],
            })),
            Tag::Item => Frame::Node(Node::ListItem(ListItem {
                span,
                checked: None,
                children: vec![],
            })),
            Tag::Table(aligns) => {
                self.aligns = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                Frame::Node(Node::Table(Table {
                    span,
                    align: self.aligns.clone(),
                    children: vec![],
                }))
            }
            Tag::TableHead => Frame::TableHead(TableRow {
                span,
                header: true,
                children: vec![],
            }),
            Tag::TableRow => Frame::Node(Node::TableRow(TableRow {
                span,
                header: false,
                children: vec![],
            })),
            Tag::TableCell => Frame::Node(Node::TableCell(TableCell {
                span,
                children: vec![],
            })),
            Tag::Emphasis => Frame::Node(Node::Emphasis(Emphasis {
                span,
                children: vec![],
            })),
            Tag::Strong => Frame::Node(Node::Strong(Strong {
                span,
                children: vec![],
            })),
            Tag::Link {
                dest_url, title, ..
            } => Frame::Node(Node::Link(Link {
                span,
                url: dest_url.into_string(),
                title: Some(title.into_string()).filter(|t| !t.is_empty()),
                target: None,
                children: vec![],
            })),
            Tag::Image {
                dest_url, title, ..
            } => Frame::Image(Image {
                span,
                url: dest_url.into_string(),
                alt: String::new(),
                title: Some(title.into_string()).filter(|t| !t.is_empty()),
            }),
            Tag::MetadataBlock(kind) => {
                Frame::Metadata(kind, span.unwrap_or(Span::new(0, 0)), String::new())
            }
            // Not enabled; keep their content by wrapping it in a paragraph-
            // like container that is flattened on close.
            Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => Frame::Node(Node::Paragraph(Paragraph {
                span,
                children: vec![],
            })),
        };
        self.stack.push(frame);
    }

    fn end(&mut self, tag: TagEnd) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Node(node) => {
                // Inline containers from unsupported extensions are
                // flattened into their parent.
                let flatten = matches!(
                    tag,
                    TagEnd::Strikethrough | TagEnd::Superscript | TagEnd::Subscript
                );
                if flatten {
                    if let Node::Paragraph(p) = node {
                        for child in p.children {
                            self.push(child);
                        }
                    }
                } else {
                    self.push(node);
                }
            }
            Frame::Code(code) => self.push(Node::CodeBlock(code)),
            Frame::Image(image) => self.push(Node::Image(image)),
            Frame::Html(raw) => self.push(Node::Raw(raw)),
            Frame::TableHead(row) => self.push(Node::TableRow(row)),
            Frame::Metadata(kind, span, text) => self.front_matter(kind, span, &text),
        }
    }

    fn front_matter(&mut self, kind: MetadataBlockKind, span: Span, text: &str) {
        let parsed: Result<Value, String> = match kind {
            MetadataBlockKind::YamlStyle => serde_saphyr::from_str(text).map_err(|e| e.to_string()),
            MetadataBlockKind::PlusesStyle => toml::from_str(text).map_err(|e| e.to_string()),
        };
        let format = match kind {
            MetadataBlockKind::YamlStyle => "YAML",
            MetadataBlockKind::PlusesStyle => "TOML",
        };
        match parsed {
            Ok(Value::Object(map)) => {
                self.document.metadata = map.into_iter().collect::<Metadata>()
            }
            Ok(Value::Null) => {}
            Ok(_) => self.warn(
                span,
                format!("{format} front matter must be a mapping of keys to values"),
            ),
            Err(e) => self.warn(span, format!("invalid {format} front matter: {e}")),
        }
    }

    fn warn(&mut self, span: Span, message: String) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            message,
            source: Some(self.document.source.clone()),
            span: Some(span),
            code: Some("front-matter".into()),
        });
    }

    fn finish(mut self) -> (Document, Vec<Diagnostic>) {
        // Close anything left open by malformed input.
        while !self.stack.is_empty() {
            self.end(TagEnd::Paragraph);
        }
        assign_heading_ids(&mut self.document);
        (self.document, self.diagnostics)
    }
}

/// Gives every heading an id: its explicit one, or a unique slug of its
/// text.
fn assign_heading_ids(doc: &mut Document) {
    let mut used: HashMap<String, usize> = HashMap::new();
    // Explicit ids are reserved first, so slugs never collide with them.
    for node in doc.descendants() {
        if let Node::Heading(Heading { id: Some(id), .. }) = node {
            used.entry(id.clone()).or_insert(0);
        }
    }
    fn visit(nodes: &mut [Node], used: &mut HashMap<String, usize>) {
        for node in nodes {
            if let Node::Heading(h) = node
                && h.id.is_none()
            {
                let base = match slug(&plain_text(&h.children)) {
                    s if s.is_empty() => "section".to_owned(),
                    s => s,
                };
                let mut id = base.clone();
                while let Some(n) = used.get_mut(&id) {
                    *n += 1;
                    id = format!("{base}-{n}");
                }
                used.insert(id.clone(), 0);
                h.id = Some(id);
            }
            if let Some(children) = node.children_mut() {
                visit(children, used);
            }
        }
    }
    visit(&mut doc.children, &mut used);
}

/// A URL-friendly identifier for a heading: lowercase letters and digits,
/// with runs of anything else replaced by `-`.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_ok(input: &str) -> Document {
        let (doc, diagnostics) = parse("t.md", input);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        doc
    }

    fn at(input: &str, node: &Node) -> String {
        let s = node.span().unwrap();
        input[s.start as usize..s.end as usize].to_owned()
    }

    #[test]
    fn block_structure_and_spans() {
        let input = "# Title\n\nSome *em* and **strong** with `code`.\n\n> quote\n\n---\n";
        let doc = parse_ok(input);
        let kinds: Vec<_> = doc.children.iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["heading", "paragraph", "block_quote", "thematic_break"]
        );
        assert_eq!(at(input, &doc.children[0]), "# Title\n");
        let para = &doc.children[1];
        assert_eq!(at(input, para), "Some *em* and **strong** with `code`.\n");
        let inline: Vec<_> = para.children().iter().map(|n| n.kind()).collect();
        assert_eq!(
            inline,
            [
                "text",
                "emphasis",
                "text",
                "strong",
                "text",
                "inline_code",
                "text"
            ]
        );
        assert_eq!(at(input, &para.children()[1]), "*em*");
    }

    #[test]
    fn yaml_and_toml_front_matter() {
        let doc = parse_ok("---\ntitle: Hello\ntags: [a, b]\nweight: 3\n---\n# Hi\n");
        assert_eq!(doc.metadata["title"], json!("Hello"));
        assert_eq!(doc.metadata["tags"], json!(["a", "b"]));
        assert_eq!(doc.metadata["weight"], json!(3));
        assert_eq!(doc.children.len(), 1);

        let doc = parse_ok("+++\ntitle = \"Plus\"\n+++\ntext\n");
        assert_eq!(doc.metadata["title"], json!("Plus"));
    }

    #[test]
    fn bad_front_matter_is_a_warning() {
        let (doc, diagnostics) = parse("t.md", "---\ntitle: [unclosed\n---\n# Hi\n");
        assert_eq!(doc.children.len(), 1);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(
            diagnostics[0]
                .message
                .starts_with("invalid YAML front matter"),
            "{diagnostics:?}"
        );
        assert_eq!(diagnostics[0].span.unwrap().start, 0);

        let (_, diagnostics) = parse("t.md", "---\n- just\n- a list\n---\n");
        assert!(
            diagnostics[0].message.contains("must be a mapping"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn heading_ids_are_unique_and_respect_explicit_ones() {
        let doc = parse_ok("# Intro\n## Intro\n## Custom {#intro-1}\n## Intro\n# `x` & y!\n");
        let ids: Vec<_> = doc
            .children
            .iter()
            .map(|n| match n {
                Node::Heading(h) => h.id.clone().unwrap(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ids, ["intro", "intro-2", "intro-1", "intro-3", "x-y"]);
    }

    #[test]
    fn code_blocks() {
        let doc = parse_ok("```rust title=\"x.rs\"\nfn main() {}\n```\n\n    indented\n");
        let Node::CodeBlock(fenced) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(fenced.language.as_deref(), Some("rust"));
        assert_eq!(fenced.info.as_deref(), Some("title=\"x.rs\""));
        assert_eq!(fenced.value, "fn main() {}\n");
        let Node::CodeBlock(indented) = &doc.children[1] else {
            panic!()
        };
        assert_eq!(
            (indented.language.as_deref(), indented.value.as_str()),
            (None, "indented\n")
        );
    }

    #[test]
    fn lists_and_task_items() {
        let doc = parse_ok("3. three\n4. four\n\n- [x] done\n- [ ] todo\n");
        let Node::List(ordered) = &doc.children[0] else {
            panic!()
        };
        assert!(ordered.ordered);
        assert_eq!(ordered.start, Some(3));
        let Node::List(tasks) = &doc.children[1] else {
            panic!()
        };
        let checked: Vec<_> = tasks
            .children
            .iter()
            .map(|n| match n {
                Node::ListItem(i) => i.checked,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(checked, [Some(true), Some(false)]);
    }

    #[test]
    fn tables() {
        let doc = parse_ok("| a | b |\n|:--|--:|\n| 1 | 2 |\n");
        let Node::Table(table) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(table.align, [Align::Left, Align::Right]);
        let rows: Vec<bool> = table
            .children
            .iter()
            .map(|r| match r {
                Node::TableRow(r) => r.header,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(rows, [true, false]);
        let Node::Text(a) = &table.children[0].children()[0].children()[0] else {
            panic!()
        };
        assert_eq!(a.value, "a");
    }

    #[test]
    fn links_images_html_and_breaks() {
        let doc = parse_ok(
            "[docs](guide.md \"Guide\") ![alt *text*](a.png)  \nnext <b>x</b>\n\n<div>\nblock\n</div>\n",
        );
        let para = doc.children[0].children();
        let Node::Link(link) = &para[0] else {
            panic!("{para:?}")
        };
        assert_eq!(
            (link.url.as_str(), link.title.as_deref()),
            ("guide.md", Some("Guide"))
        );
        let Node::Image(image) = &para[2] else {
            panic!("{para:?}")
        };
        assert_eq!(
            (image.url.as_str(), image.alt.as_str()),
            ("a.png", "alt text")
        );
        assert!(para.iter().any(|n| matches!(n, Node::LineBreak(_))));
        assert!(
            para.iter()
                .any(|n| matches!(n, Node::Raw(r) if r.value == "<b>"))
        );
        let Node::Raw(block) = &doc.children[1] else {
            panic!()
        };
        assert_eq!(block.value, "<div>\nblock\n</div>\n");
    }

    #[test]
    fn soft_breaks_merge_into_text() {
        let doc = parse_ok("one\ntwo\n");
        assert_eq!(
            doc.children[0].children(),
            [Node::Text(Text {
                span: Some(Span::new(0, 7)),
                value: "one\ntwo".into()
            })]
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Getting Started!"), "getting-started");
        assert_eq!(slug("  Ünïcode & more  "), "ünïcode-more");
    }
}
