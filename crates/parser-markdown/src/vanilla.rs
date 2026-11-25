//! The `md-vanilla` dialect: the `parser-md-vanilla` plugin.
//!
//! Parses Markdown as the `pulldown-cmark` library reads it (CommonMark
//! plus its extensions) into a document tree in the shared shape (see
//! "Document shape" in `spec/wire.md`). Every node carries the byte span
//! it came from.
//!
//! - Top-level headings open sections: everything up to the next heading of
//!   the same or a higher level is nested in a `section` whose first child
//!   is the heading. Headings inside block quotes or lists stay headings.
//! - Extensions: GitHub tables, task lists and alerts (`> [!NOTE]` becomes
//!   an `admonition`), footnotes (`[^1]`), definition lists, and math
//!   (`$x$` inline, `$$x$$` display).
//! - Front matter: YAML between `---` fences, or TOML between `+++` fences,
//!   at the very start of the file. It must be a mapping; it becomes the
//!   document's metadata.
//! - An explicit `{#id}` on a heading becomes its section's `id`; the host
//!   generates the others after parsing.
//! - Raw HTML becomes `raw` nodes with format `html`.
//! - Soft line breaks become `\n` within text; hard breaks become
//!   `line_break` nodes.
//! - No syntax extensions: a fence whose language is `{name}` stays a code
//!   block, and `` {name}`text` `` stays text and code.
//!
//! Settings, in `.config/lsdocs/<name>/settings.toml` (`name` being the
//! plugin's name in the site config):
//!
//! - `smart-punctuation` (boolean, default `false`, as docutils'
//!   `smart_quotes`): curly quotes, en and em dashes, and ellipses.

use std::ops::Range;

use lsdocs_plugin_sdk::config::{self, Kind};
use lsdocs_plugin_sdk::doctree::{
    Admonition, Align, BlockQuote, CodeBlock, DefinitionDescription, DefinitionList,
    DefinitionTerm, Document, Emphasis, FootnoteDefinition, FootnoteReference, Heading, Image,
    InlineCode, LineBreak, Link, List, ListItem, Math, Metadata, Node, Paragraph, Raw, Section,
    Span, Strong, Table, TableCell, TableRow, Text, ThematicBreak, Value,
};
use lsdocs_plugin_sdk::protocol::{Diagnostic, ParseParams, ParseResult, Severity};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Options, Parser as Cmark,
    Tag, TagEnd,
};

/// The `smart-punctuation` setting's key.
const SMART_PUNCTUATION: &str = "smart-punctuation";

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-md-vanilla", env!("CARGO_PKG_VERSION"))
        .parser(Vanilla)
        .settings(|c| config::settings_toml_keys(c, &[(SMART_PUNCTUATION, Kind::Bool)]))
        .cacheable()
}

/// The plugin's settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// Curly quotes, dashes and ellipses (`smart-punctuation`).
    pub smart_punctuation: bool,
}

impl Settings {
    /// The settings in `value`, as the plugin's loader read them (null or
    /// a missing key being the default).
    pub fn from_value(value: &Value) -> Self {
        Self {
            smart_punctuation: value
                .get(SMART_PUNCTUATION)
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        }
    }
}

/// The `md-vanilla` parser.
#[derive(Debug, Clone, Copy, Default)]
pub struct Vanilla;

impl Parser for Vanilla {
    fn extensions(&self) -> Vec<String> {
        vec!["md".into(), "markdown".into()]
    }

    async fn parse(&self, params: ParseParams, ctx: &Context) -> PluginResult<ParseResult> {
        let settings = Settings::from_value(ctx.settings());
        let (document, diagnostics) = parse(&params.source, &params.text, &settings);
        Ok(ParseResult {
            document,
            diagnostics,
            dependencies: Vec::new(),
        })
    }
}

fn options(settings: &Settings) -> Options {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_MATH
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    if settings.smart_punctuation {
        options | Options::ENABLE_SMART_PUNCTUATION
    } else {
        options
    }
}

/// Parses Markdown into a document, plus any problems found (e.g. invalid
/// front matter).
pub fn parse(source: &str, input: &str, settings: &Settings) -> (Document, Vec<Diagnostic>) {
    let mut builder = Builder::new(source);
    for (event, range) in Cmark::new_ext(input, options(settings)).into_offset_iter() {
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
        let event_kind = match event {
            Event::DisplayMath(_) => MathKind::Display,
            _ => MathKind::Inline,
        };
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
            Event::InlineMath(value) | Event::DisplayMath(value) => self.push(Node::Math(Math {
                span: span(&range),
                display: matches!(event_kind, MathKind::Display),
                value: value.into_string(),
            })),
            Event::FootnoteReference(label) => {
                self.push(Node::FootnoteReference(FootnoteReference {
                    span: span(&range),
                    label: label.into_string(),
                }))
            }
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
            Tag::BlockQuote(None) => Frame::Node(Node::BlockQuote(BlockQuote {
                span,
                children: vec![],
            })),
            // A GitHub alert: `> [!NOTE]`.
            Tag::BlockQuote(Some(kind)) => Frame::Node(Node::Admonition(Admonition {
                span,
                kind: match kind {
                    BlockQuoteKind::Note => "note",
                    BlockQuoteKind::Tip => "tip",
                    BlockQuoteKind::Important => "important",
                    BlockQuoteKind::Warning => "warning",
                    BlockQuoteKind::Caution => "caution",
                }
                .into(),
                title: None,
                children: vec![],
            })),
            Tag::FootnoteDefinition(label) => {
                Frame::Node(Node::FootnoteDefinition(FootnoteDefinition {
                    span,
                    label: label.into_string(),
                    children: vec![],
                }))
            }
            Tag::DefinitionList => Frame::Node(Node::DefinitionList(DefinitionList {
                span,
                children: vec![],
            })),
            Tag::DefinitionListTitle => Frame::Node(Node::DefinitionTerm(DefinitionTerm {
                span,
                children: vec![],
            })),
            Tag::DefinitionListDefinition => {
                Frame::Node(Node::DefinitionDescription(DefinitionDescription {
                    span,
                    children: vec![],
                }))
            }
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
            Tag::Strikethrough | Tag::Superscript | Tag::Subscript => {
                Frame::Node(Node::Paragraph(Paragraph {
                    span,
                    children: vec![],
                }))
            }
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
        let children = std::mem::take(&mut self.document.children);
        self.document.children = sectionize(children);
        (self.document, self.diagnostics)
    }
}

/// Whether a math event is inline or display.
enum MathKind {
    Inline,
    Display,
}

/// Nests top-level nodes into sections: each heading opens a section that
/// holds everything up to the next heading of the same or a higher level
/// (a lower or equal level number). The heading's id moves to the section.
fn sectionize(nodes: Vec<Node>) -> Vec<Node> {
    fn add(section: &mut Section, node: Node) {
        if let (Some(span), Some(end)) = (&mut section.span, node.span()) {
            span.end = span.end.max(end.end);
        }
        section.children.push(node);
    }
    fn close(open: &mut Vec<(u8, Section)>, out: &mut Vec<Node>) {
        if let Some((_, section)) = open.pop() {
            let node = Node::Section(section);
            match open.last_mut() {
                Some((_, parent)) => add(parent, node),
                None => out.push(node),
            }
        }
    }
    let mut out = Vec::new();
    let mut open: Vec<(u8, Section)> = Vec::new();
    for node in nodes {
        match node {
            Node::Heading(mut heading) => {
                while open
                    .last()
                    .is_some_and(|(level, _)| *level >= heading.level)
                {
                    close(&mut open, &mut out);
                }
                let level = heading.level;
                let mut section = Section {
                    span: heading.span,
                    id: heading.id.take(),
                    children: vec![],
                };
                add(&mut section, Node::Heading(heading));
                open.push((level, section));
            }
            node => match open.last_mut() {
                Some((_, section)) => add(section, node),
                None => out.push(node),
            },
        }
    }
    while !open.is_empty() {
        close(&mut open, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use lsdocs_plugin_sdk::protocol::{ConfigInfo, InitializeParams};
    use serde_json::json;

    fn parse_with(input: &str, settings: &Settings) -> Document {
        let (doc, diagnostics) = parse("t.md", input, settings);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        doc
    }

    fn parse_ok(input: &str) -> Document {
        parse_with(input, &Settings::default())
    }

    fn at(input: &str, node: &Node) -> String {
        let s = node.span().unwrap();
        input[s.start as usize..s.end as usize].to_owned()
    }

    /// The kinds of `nodes` and their descendants, depth first.
    fn kinds(nodes: &[Node]) -> Vec<String> {
        let mut out = Vec::new();
        for node in nodes {
            out.push(node.kind().into_owned());
            out.extend(kinds(node.children()));
        }
        out
    }

    /// The text of `nodes` and their descendants, concatenated.
    fn text(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|n| match n {
                Node::Text(t) => t.value.clone(),
                n => text(n.children()),
            })
            .collect()
    }

    /// The plugin's setup with `settings` as its settings file (none if
    /// `None`), in a directory of its own named `tag`.
    fn setup(tag: &str, settings: Option<&str>) -> Result<Value, String> {
        let dir =
            std::env::temp_dir().join(format!("lsdocs-md-vanilla-{tag}-{}", std::process::id()));
        if let Some(text) = settings {
            std::fs::create_dir_all(dir.join("lsdocs/md")).unwrap();
            std::fs::write(dir.join("lsdocs/md/settings.toml"), text).unwrap();
        }
        let params = InitializeParams {
            config: ConfigInfo {
                dir: dir.to_string_lossy().into_owned(),
                name: "md".into(),
            },
            ..InitializeParams::default()
        };
        let result = plugin().setup(&params).map(|s| s.settings);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn block_structure_and_spans() {
        let input = "# Title\n\nSome *em* and **strong** with `code`.\n\n> quote\n\n---\n";
        let doc = parse_ok(input);
        let [Node::Section(section)] = &doc.children[..] else {
            panic!("{doc:?}")
        };
        let kinds: Vec<_> = section.children.iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["heading", "paragraph", "block_quote", "thematic_break"]
        );
        assert_eq!(at(input, &doc.children[0]), input);
        let para = &section.children[1];
        assert_eq!(at(input, para), "Some *em* and **strong** with `code`.\n");
        assert_eq!(at(input, &para.children()[1]), "*em*");
    }

    #[test]
    fn extensions() {
        let input = "| a | b |\n|:--|--:|\n| 1 | 2 |\n\n- [x] done\n\n> [!NOTE]\n> n\n\n\
                     x[^1]\n\n[^1]: f\n\nterm\n: def\n\n$x$ and\n\n$$y$$\n";
        let found = kinds(&parse_ok(input).children);
        for kind in [
            "table",
            "table_row",
            "table_cell",
            "list_item",
            "admonition",
            "footnote_reference",
            "footnote_definition",
            "definition_list",
            "definition_term",
            "definition_description",
            "math",
        ] {
            assert!(found.iter().any(|k| k == kind), "{kind} not in {found:?}");
        }
    }

    #[test]
    fn heading_id_moves_to_section() {
        let doc = parse_ok("# Title {#top}\n");
        let [Node::Section(section)] = &doc.children[..] else {
            panic!("{doc:?}")
        };
        assert_eq!(section.id.as_deref(), Some("top"));
    }

    #[test]
    fn front_matter() {
        let doc = parse_ok("---\ntitle: Hello\n---\n# Hi\n");
        assert_eq!(doc.metadata["title"], json!("Hello"));
        let doc = parse_ok("+++\ntitle = \"Plus\"\n+++\ntext\n");
        assert_eq!(doc.metadata["title"], json!("Plus"));
        let (_, diagnostics) = parse("t.md", "---\n[1]\n---\n", &Settings::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
    }

    #[test]
    fn myst_syntax_is_plain_markdown() {
        let doc = parse_ok("```{note} Title\nbody\n```\n\nA {ref}`x` role.\n");
        let Node::CodeBlock(code) = &doc.children[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(code.language.as_deref(), Some("{note}"));
        assert_eq!(code.info.as_deref(), Some("Title"));
        assert_eq!(code.value, "body\n");
        let inline: Vec<_> = doc.children[1].children().iter().map(Node::kind).collect();
        assert_eq!(inline, ["text", "inline_code", "text"]);
        assert_eq!(text(&doc.children[1..]), "A {ref} role.");
    }

    #[test]
    fn smart_punctuation_is_off_by_default() {
        let doc = parse_ok("\"a\" -- b...\n");
        assert_eq!(text(&doc.children), "\"a\" -- b...");
        assert_eq!(Settings::from_value(&Value::Null), Settings::default());
    }

    #[test]
    fn smart_punctuation_when_set() {
        let settings =
            Settings::from_value(&setup("on", Some("smart-punctuation = true\n")).unwrap());
        assert!(settings.smart_punctuation);
        let doc = parse_with("\"a\" -- b...\n", &settings);
        assert_eq!(text(&doc.children), "\u{201c}a\u{201d} \u{2013} b\u{2026}");
    }

    #[test]
    fn bad_settings_fail_setup() {
        assert_eq!(setup("none", None).unwrap(), Value::Null);
        let err = setup("type", Some("smart-punctuation = \"yes\"\n")).unwrap_err();
        assert!(
            err.contains("settings.toml") && err.contains("a boolean"),
            "{err}"
        );
        let err = setup("key", Some("smart-quotes = true\n")).unwrap_err();
        assert!(err.contains("unknown setting `smart-quotes`"), "{err}");
    }
}
