//! The MyST Markdown parser plugin, `parser-myst`.
//!
//! Parses CommonMark (via `pulldown-cmark`), with MyST's directives and
//! roles, into a document tree in the
//! shared shape (see "Document shape" in `spec/wire.md`). Every node carries
//! the byte span it came from.
//!
//! - Top-level headings open sections: everything up to the next heading of
//!   the same or a higher level is nested in a `section` whose first child
//!   is the heading. Headings inside block quotes or lists stay headings.
//! - Extensions: GitHub tables, task lists and alerts (`> [!NOTE]` becomes
//!   an `admonition`), footnotes (`[^1]`), definition lists, and math
//!   (`$x$` inline, `$$x$$` display).
//!
//! - Front matter: YAML between `---` fences, or TOML between `+++` fences,
//!   at the very start of the file. It must be a mapping; it becomes the
//!   document's metadata.
//! - An explicit `{#id}` on a heading becomes its section's `id`; the host
//!   generates the others after parsing.
//! - Raw HTML becomes `raw` nodes with format `html`.
//! - Soft line breaks become `\n` within text; hard breaks become
//!   `line_break` nodes.
//! - Directives and roles, in MyST's syntax: a fence whose language is
//!   `{name}` is a directive (the rest of the info line is its argument,
//!   leading `:option: value` lines or a `---` YAML block its options,
//!   the rest its content), and `` {name}`text` `` is a role. Those
//!   plugins declare (see "Syntax extensions" in `spec/wire.md`) are read
//!   by their declaration: options are checked and markup content is
//!   parsed. Other directives become custom nodes of their name with the
//!   content as text, as unknown reStructuredText directives do; other
//!   roles stay text and code.

mod convert;
mod md;
mod plugins;

use std::collections::BTreeMap;
use std::ops::Range;

use convert::sectionize;
use lsdocs_plugin_sdk::doctree::{
    Admonition, Align, BlockQuote, CodeBlock, Custom, DefinitionDescription, DefinitionList,
    DefinitionTerm, Document, Emphasis, FootnoteDefinition, FootnoteReference, Heading, Image,
    InlineCode, LineBreak, Link, List, ListItem, Math, Metadata, Node, Paragraph, Raw, Span,
    Strong, Table, TableCell, TableRow, Text, ThematicBreak, Value,
};
use lsdocs_plugin_sdk::protocol::{
    ArgumentSyntax, ContentSyntax, Diagnostic, DirectiveSyntax, OptionSyntax, ParseParams,
    ParseResult, Severity, Syntax,
};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Options, Parser as Cmark,
    Tag, TagEnd,
};

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-myst", env!("CARGO_PKG_VERSION"))
        .parser(Myst)
        .cacheable()
}

/// The MyST parser.
#[derive(Debug, Clone, Copy, Default)]
pub struct Myst;

impl Parser for Myst {
    fn extensions(&self) -> Vec<String> {
        vec!["md".into(), "markdown".into()]
    }

    async fn parse(&self, params: ParseParams, _ctx: &Context) -> PluginResult<ParseResult> {
        let (document, diagnostics) =
            parse_with_syntax(&params.source, &params.text, &params.syntax);
        Ok(ParseResult {
            document,
            diagnostics,
            dependencies: Vec::new(),
        })
    }
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_MATH
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

/// Parses Markdown into a document, plus any problems found (e.g. invalid
/// front matter).
pub fn parse(source: &str, input: &str) -> (Document, Vec<Diagnostic>) {
    parse_with_syntax(source, input, &Syntax::default())
}

/// Parses Markdown as [`parse`] does, reading the directives and roles of
/// `syntax` by their declarations.
pub fn parse_with_syntax(
    source: &str,
    input: &str,
    syntax: &Syntax,
) -> (Document, Vec<Diagnostic>) {
    parse_part(source, input, syntax, options())
}

/// Parses `input` with markdown-it (the pipeline replacing the
/// pulldown-cmark one; see the plan in docs/superpowers/plans).
#[cfg_attr(not(test), allow(dead_code))]
fn parse_mdit(source: &str, input: &str, syntax: &Syntax) -> (Document, Vec<Diagnostic>) {
    let root = md::parser(syntax).parse(input);
    convert::document(source, input, &root)
}

/// Parses `input`, the whole file or a piece of it, with `options`.
fn parse_part(
    source: &str,
    input: &str,
    syntax: &Syntax,
    options: Options,
) -> (Document, Vec<Diagnostic>) {
    let mut builder = Builder::new(source, input, syntax);
    for (event, range) in Cmark::new_ext(input, options).into_offset_iter() {
        builder.event(event, range);
    }
    builder.finish()
}

/// Parses a piece of the file (a directive's content or argument) found
/// at `map`'s offsets: no front matter, and spans moved to where the
/// piece is in the file.
fn parse_piece(
    source: &str,
    piece: &str,
    syntax: &Syntax,
    map: &impl Fn(u32) -> u32,
) -> (Vec<Node>, Vec<Diagnostic>) {
    let options = options()
        - Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        - Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    let (mut doc, mut diagnostics) = parse_part(source, piece, syntax, options);
    move_spans(&mut doc.children, map);
    for d in &mut diagnostics {
        if let Some(s) = &mut d.span {
            *s = Span::new(map(s.start), map(s.end));
        }
    }
    (doc.children, diagnostics)
}

/// Moves the spans of `nodes` and their descendants by `map`.
fn move_spans(nodes: &mut [Node], map: &impl Fn(u32) -> u32) {
    for node in nodes {
        if let Some(s) = node.span_mut() {
            *s = Span::new(map(s.start), map(s.end));
        }
        if let Some(children) = node.children_mut() {
            move_spans(children, map);
        }
    }
}

/// What an open element is collecting.
enum Frame {
    /// A node whose children are collected into it.
    Node(Node),
    /// A code block; text is its value. Each piece of text's offset in
    /// the value and in the source are noted, to find directive content
    /// in the source.
    Code(CodeBlock, Vec<(usize, usize)>),
    /// An image; text inside is its alt text.
    Image(Image),
    /// Raw HTML; text is its value.
    Html(Raw),
    /// Front matter; text is its source.
    Metadata(MetadataBlockKind, Span, String),
    /// A table head: cells collected into a header row.
    TableHead(TableRow),
}

struct Builder<'a> {
    document: Document,
    /// The text being parsed.
    input: &'a str,
    /// Declared directives and roles.
    syntax: &'a Syntax,
    stack: Vec<Frame>,
    diagnostics: Vec<Diagnostic>,
    /// Column alignments of the table being built.
    aligns: Vec<Align>,
}

fn span(range: &Range<usize>) -> Option<Span> {
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Some(Span::new(clamp(range.start), clamp(range.end)))
}

impl<'a> Builder<'a> {
    fn new(source: &str, input: &'a str, syntax: &'a Syntax) -> Self {
        Self {
            document: Document::new(source),
            input,
            syntax,
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
            Some(Frame::Code(code, pieces)) => {
                pieces.push((code.value.len(), range.start));
                code.value.push_str(value);
            }
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
                None => match self.role_name(range.start) {
                    Some((name, start)) => {
                        let span = span(&(start..range.end));
                        let value = code.into_string();
                        self.push(Node::Custom(Custom {
                            span,
                            kind: "role".into(),
                            attrs: BTreeMap::from([
                                ("name".into(), Value::String(name)),
                                ("text".into(), Value::String(value.clone())),
                            ]),
                            children: vec![Node::Text(Text { span, value })],
                        }));
                    }
                    None => self.push(Node::InlineCode(InlineCode {
                        span: span(&range),
                        value: code.into_string(),
                    })),
                },
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
                Frame::Code(
                    CodeBlock {
                        span,
                        language,
                        info,
                        value: String::new(),
                    },
                    Vec::new(),
                )
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
            Frame::Code(code, pieces) => {
                let node = self.code_or_directive(code, &pieces);
                self.push(node);
            }
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
        self.warn_with(Some(span), message, "front-matter");
    }

    fn warn_with(&mut self, span: Option<Span>, message: String, code: &str) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            message,
            source: Some(self.document.source.clone()),
            span,
            code: Some(code.into()),
        });
    }

    /// If the inline code starting at `start` follows a declared role's
    /// `{name}`, removes that from the text before it and returns the
    /// role's name and where its `{` is.
    fn role_name(&mut self, start: usize) -> Option<(String, usize)> {
        let syntax = self.syntax;
        let children = match self.stack.last_mut() {
            Some(Frame::Node(parent)) => parent.children_mut()?,
            Some(Frame::TableHead(row)) => &mut row.children,
            _ => return None,
        };
        let Some(Node::Text(text)) = children.last_mut() else {
            return None;
        };
        let end = text.span?.end as usize;
        if end != start {
            return None;
        }
        let rest = text.value.strip_suffix('}')?;
        let open = rest.rfind('{')?;
        let name = &rest[open + 1..];
        if !is_name(name) || !syntax.has_role(&name.to_lowercase()) {
            return None;
        }
        let name = name.to_lowercase();
        let len = text.value.len() - open;
        let brace = start - len;
        text.value.truncate(open);
        if let Some(s) = &mut text.span {
            s.end = u32::try_from(brace).unwrap_or(s.end);
        }
        if text.value.is_empty() {
            children.pop();
        }
        Some((name, brace))
    }

    /// A fenced code block, or a directive if its language is `{name}`.
    fn code_or_directive(&mut self, code: CodeBlock, pieces: &[(usize, usize)]) -> Node {
        let Some(name) = code
            .language
            .as_deref()
            .and_then(|l| l.strip_prefix('{')?.strip_suffix('}'))
            .filter(|n| is_name(n))
            .map(str::to_lowercase)
        else {
            return Node::CodeBlock(code);
        };
        let at = code.span.map(|s| s.start);
        // Where the value's offsets are in the source.
        let source_offset = |offset: usize| -> u32 {
            let found = pieces.iter().rev().find(|(value, _)| *value <= offset);
            let n = match found {
                Some((value, source)) => source + (offset - value),
                None => at.map_or(0, |a| a as usize),
            };
            u32::try_from(n).unwrap_or(u32::MAX)
        };
        let argument = code.info.clone().unwrap_or_default();
        let (options, body_start, options_error) = directive_options(&code.value);
        let body = code.value[body_start..].trim_end_matches('\n');
        let syntax = self.syntax;
        let mut attrs = BTreeMap::new();
        if !argument.is_empty() {
            attrs.insert("argument".to_owned(), Value::String(argument.clone()));
        }
        let mut children = Vec::new();
        let span = code.span;
        if let Some(error) = options_error {
            self.warn_with(span, format!("directive \"{name}\": {error}"), "directive");
        }
        let Some(declared) = syntax.directive(&name) else {
            // Not declared: kept as text, as reStructuredText keeps
            // unknown directives.
            let options: serde_json::Map<String, Value> = options
                .into_iter()
                .map(|(k, v)| (k, Value::String(v.unwrap_or_default())))
                .collect();
            if !options.is_empty() {
                attrs.insert("options".to_owned(), Value::Object(options));
            }
            if !body.is_empty() {
                attrs.insert("content".to_owned(), Value::String(body.to_owned()));
            }
            return Node::Custom(Custom {
                span,
                kind: name,
                attrs,
                children,
            });
        };
        let mut problems = Vec::new();
        check_arguments(declared, &argument, &mut problems);
        let mut values = serde_json::Map::new();
        for (key, value) in options {
            match (declared.options.get(&key), value) {
                (None, _) => problems.push(format!("unknown option: \"{key}\"")),
                (Some(OptionSyntax::Flag), Some(v)) if !v.is_empty() => {
                    problems.push(format!("option \"{key}\" takes no value"))
                }
                (Some(OptionSyntax::Flag), _) => {
                    values.insert(key, Value::Bool(true));
                }
                (Some(OptionSyntax::Required), None) => {
                    problems.push(format!("option \"{key}\" needs a value"))
                }
                (Some(OptionSyntax::Required), Some(v)) if v.is_empty() => {
                    problems.push(format!("option \"{key}\" needs a value"))
                }
                (Some(_), v) => {
                    values.insert(key, Value::String(v.unwrap_or_default()));
                }
            }
        }
        if !values.is_empty() {
            attrs.insert("options".to_owned(), Value::Object(values));
        }
        match declared.content {
            ContentSyntax::None if !body.trim().is_empty() => {
                problems.push("no content permitted".to_owned());
            }
            ContentSyntax::None => {}
            ContentSyntax::Text => {
                if !body.is_empty() {
                    attrs.insert("content".to_owned(), Value::String(body.to_owned()));
                }
            }
            ContentSyntax::Blocks => {
                let map = |o: u32| source_offset(body_start + o as usize);
                let (nodes, diagnostics) = parse_piece(&self.document.source, body, syntax, &map);
                children = nodes;
                self.diagnostics.extend(diagnostics);
            }
        }
        if declared.argument == ArgumentSyntax::Inline && !argument.is_empty() {
            // The argument is on the fence line, after `{name}`.
            let line_start = span.map_or(0, |s| s.start as usize);
            let line = self.input[line_start.min(self.input.len())..]
                .lines()
                .next()
                .unwrap_or_default();
            let offset = line.find(argument.as_str()).map_or(0, |i| line_start + i);
            let map = |o: u32| u32::try_from(offset).unwrap_or(0).saturating_add(o);
            let (nodes, diagnostics) = parse_piece(&self.document.source, &argument, syntax, &map);
            self.diagnostics.extend(diagnostics);
            let inlines = match nodes.into_iter().next() {
                Some(Node::Paragraph(p)) => p.children,
                _ => Vec::new(),
            };
            let end = offset + argument.len();
            children.insert(
                0,
                Node::Custom(Custom {
                    span: Some(Span::new(map(0), u32::try_from(end).unwrap_or(0))),
                    kind: "directive_argument".into(),
                    attrs: BTreeMap::new(),
                    children: inlines,
                }),
            );
        }
        for problem in problems {
            self.warn_with(
                span,
                format!("directive \"{name}\": {problem}"),
                "directive",
            );
        }
        Node::Custom(Custom {
            span,
            kind: name,
            attrs,
            children,
        })
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

/// Whether `name` can be a directive or role name: letters, digits and
/// `-`, `_`, `:`, `.`, `+`, starting with a letter or digit.
fn is_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_alphanumeric)
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '+'))
}

/// Directive options as written: names and values (`None` for none).
type OptionList = Vec<(String, Option<String>)>;

/// A directive's options at the start of its fence's text: `:name: value`
/// lines, or a YAML mapping between `---` lines. Returns them (`None` for
/// no value), where the content starts (after one blank line), and a
/// problem with them, if any.
fn directive_options(text: &str) -> (OptionList, usize, Option<String>) {
    let mut options = Vec::new();
    let mut pos = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    let mut error = None;
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        let first = lines.next().unwrap_or_default();
        let mut yaml = String::new();
        let mut end = None;
        let mut at = first.len();
        for line in lines.by_ref() {
            at += line.len();
            if line.trim_end() == "---" {
                end = Some(at);
                break;
            }
            yaml.push_str(line);
        }
        match end {
            Some(end) => {
                pos = end;
                match serde_saphyr::from_str::<Value>(&yaml) {
                    Ok(Value::Object(map)) => {
                        for (key, value) in map {
                            let value = match value {
                                Value::Null | Value::Bool(true) => None,
                                Value::String(s) => Some(s),
                                other => Some(other.to_string()),
                            };
                            options.push((key, value));
                        }
                    }
                    Ok(Value::Null) => {}
                    Ok(_) => error = Some("options must be a mapping".to_owned()),
                    Err(e) => error = Some(format!("invalid options: {e}")),
                }
            }
            None => return (options, 0, None),
        }
    } else {
        for line in lines.by_ref() {
            let Some(rest) = line.trim_end().strip_prefix(':') else {
                break;
            };
            let Some((key, value)) = rest.split_once(':') else {
                break;
            };
            if key.is_empty() || key.contains(char::is_whitespace) {
                break;
            }
            let value = value.trim();
            options.push((
                key.to_owned(),
                (!value.is_empty()).then(|| value.to_owned()),
            ));
            pos += line.len();
        }
    }
    // One blank line may separate options from content.
    if !options.is_empty()
        && let Some(line) = text[pos..].split_inclusive('\n').next()
        && line.trim().is_empty()
    {
        pos += line.len();
    }
    (options, pos, error)
}

/// Problems with a directive's argument against its declaration.
fn check_arguments(declared: &DirectiveSyntax, argument: &str, problems: &mut Vec<String>) {
    let most = declared.required_arguments + declared.optional_arguments;
    let words = argument.split_whitespace().count();
    if words < declared.required_arguments as usize {
        problems.push(format!(
            "{} argument(s) required, {words} supplied",
            declared.required_arguments
        ));
    } else if most == 0 && words > 0 {
        problems.push("no arguments permitted".to_owned());
    } else if !declared.final_argument_whitespace && words > most as usize {
        problems.push(format!(
            "maximum {most} argument(s) allowed, {words} supplied"
        ));
    }
}

/// Whether a math event is inline or display.
enum MathKind {
    Inline,
    Display,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use serde_json::json;

    /// Asserts both pipelines read `input` alike, with `syntax`.
    fn same_with(input: &str, syntax: &Syntax) {
        let old = parse_with_syntax("t.md", input, syntax);
        let new = parse_mdit("t.md", input, syntax);
        assert_eq!(old, new, "input: {input:?}");
    }

    /// [`same_with`], with no declared syntax.
    fn same(input: &str) {
        same_with(input, &Syntax::default());
    }

    #[test]
    fn same_core() {
        for input in [
            "# Title\n\nSome *em* and **strong** with `code`.\n\n> quote\n\n---\n",
            "---\ntitle: Hello\ntags: [a, b]\nweight: 3\n---\n# Hi\n",
            "+++\ntitle = \"Plus\"\n+++\ntext\n",
            "---\ntitle: [unclosed\n---\n# Hi\n",
            "---\n- just\n- a list\n---\n",
            "intro\n\n# A\n\na\n\n## B\n\n#### D\n\n### C\n\n# E\n\n> ## Quoted\n",
            "```rust title=\"x.rs\"\nfn main() {}\n```\n\n    indented\n",
            "3. three\n4. four\n\n- [x] done\n- [ ] todo\n",
            "| a | b |\n|:--|--:|\n| 1 | 2 |\n",
            "[docs](guide.md \"Guide\") ![alt *text*](a.png)  \nnext <b>x</b>\n\n<div>\nblock\n</div>\n",
            "one\ntwo\n",
        ] {
            same(input);
        }
    }

    #[test]
    fn same_footnotes_and_math() {
        for input in [
            "See[^n] $a+b$ and\n$$x^2$$\n\n[^n]: The note.\n",
            "x[^a]\n\n[^a]: First.\n\n    Second.\n",
            "Undefined[^none] here.\n",
            "$$\nx^2\n$$\n",
            "Price $5 and $ x $ no.\n",
        ] {
            same(input);
        }
    }

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
        let [Node::Section(section)] = &doc.children[..] else {
            panic!("{doc:?}")
        };
        let kinds: Vec<_> = section.children.iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["heading", "paragraph", "block_quote", "thematic_break"]
        );
        // The section spans its heading through its last child.
        assert_eq!(at(input, &doc.children[0]), input);
        assert_eq!(at(input, &section.children[0]), "# Title\n");
        let para = &section.children[1];
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

    /// Sections as `title(id)[subsections]`, with `-` for no id.
    fn outline(nodes: &[Node]) -> String {
        nodes
            .iter()
            .filter_map(|n| match n {
                Node::Section(s) => {
                    let Some(Node::Heading(h)) = s.children.first() else {
                        panic!("a section starts with its heading: {s:?}")
                    };
                    assert_eq!(h.id, None, "the id belongs to the section");
                    let inner = outline(&s.children[1..]);
                    let inner = if inner.is_empty() {
                        inner
                    } else {
                        format!("[{inner}]")
                    };
                    Some(format!(
                        "{}({}){inner}",
                        plain(&h.children),
                        s.id.as_deref().unwrap_or("-")
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn plain(nodes: &[Node]) -> String {
        lsdocs_plugin_sdk::doctree::plain_text(nodes)
    }

    #[test]
    fn headings_open_nested_sections() {
        let input = "intro\n\n# A\n\na\n\n## B {#bee}\n\n#### D\n\n### C\n\n# E\n\n> ## Quoted\n";
        let doc = parse_ok(input);
        assert_eq!(doc.children[0].kind(), "paragraph");
        assert_eq!(outline(&doc.children), "A(-)[B(bee)[D(-) C(-)]] E(-)");
        // A heading in a block quote is not a section.
        let Node::Section(e) = &doc.children[2] else {
            panic!()
        };
        assert_eq!(e.children[1].children()[0].kind(), "heading");
        assert_eq!(
            at(input, &doc.children[1]),
            "# A\n\na\n\n## B {#bee}\n\n#### D\n\n### C\n"
        );
    }

    #[test]
    fn alerts_footnotes_definitions_and_math() {
        let input = "> [!WARNING]\n> Careful.\n\nSee[^n] $a+b$ and\n$$x^2$$\n\nTerm\n: Meaning\n\n[^n]: The note.\n";
        let doc = parse_ok(input);
        let kinds: Vec<_> = doc.descendants().map(|n| n.kind().into_owned()).collect();
        for kind in [
            "admonition",
            "footnote_reference",
            "math",
            "definition_list",
            "definition_term",
            "definition_description",
            "footnote_definition",
        ] {
            assert!(kinds.iter().any(|k| k == kind), "{kind} missing: {kinds:?}");
        }
        let Node::Admonition(alert) = &doc.children[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(alert.kind, "warning");
        assert_eq!(plain(&alert.children), "Careful.");
        let maths: Vec<_> = doc
            .descendants()
            .filter_map(|n| match n {
                Node::Math(m) => Some((m.value.as_str(), m.display)),
                _ => None,
            })
            .collect();
        assert_eq!(maths, [("a+b", false), ("x^2", true)]);
        let label = doc.descendants().find_map(|n| match n {
            Node::FootnoteDefinition(f) => Some(f.label.as_str()),
            _ => None,
        });
        assert_eq!(label, Some("n"));
    }

    fn syntax() -> Syntax {
        serde_json::from_value(json!({
            "directives": [
                {"name": "versionadded", "required_arguments": 1,
                 "final_argument_whitespace": true,
                 "options": {"class": "value", "hidden": "flag"}, "content": "blocks"},
                {"name": "centered", "required_arguments": 1,
                 "final_argument_whitespace": true, "argument": "inline"},
                {"name": "code-block", "optional_arguments": 1, "content": "text"},
            ],
            "roles": [{"name": "kbd"}],
        }))
        .unwrap()
    }

    #[test]
    fn declared_directives_and_roles_become_custom_nodes() {
        let input = "```{versionadded} 1.2 beta\n:class: x\n:hidden:\n\n*New* here.\n```\n\n\
                     ```{centered} Hello *world*\n```\n\n\
                     ```{code-block} rust\nfn main() {}\n```\n\n\
                     Press {kbd}`Ctrl+C` or `code`.\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let value = serde_json::to_value(&doc.children).unwrap();
        assert_eq!(value[0]["kind"], "versionadded");
        assert_eq!(
            value[0]["attrs"],
            json!({"argument": "1.2 beta", "options": {"class": "x", "hidden": true}})
        );
        // Content is parsed, with spans in the file.
        let Node::Custom(added) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(at(input, &added.children[0]), "*New* here.");
        assert_eq!(at(input, &added.children[0].children()[0]), "*New*");
        assert_eq!(value[1]["kind"], "centered");
        let Node::Custom(centered) = &doc.children[1] else {
            panic!()
        };
        let argument = &centered.children[0];
        assert_eq!(value[1]["children"][0]["kind"], "directive_argument");
        assert_eq!(at(input, argument), "Hello *world*");
        assert_eq!(at(input, &argument.children()[1]), "*world*");
        assert_eq!(
            value[2]["attrs"],
            json!({"argument": "rust", "content": "fn main() {}"})
        );
        let para = &doc.children[3];
        let kinds: Vec<_> = para.children().iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["text", "custom:role", "text", "inline_code", "text"]
        );
        assert_eq!(at(input, &para.children()[1]), "{kbd}`Ctrl+C`");
        assert_eq!(
            value[3]["children"][1]["attrs"],
            json!({"name": "kbd", "text": "Ctrl+C"})
        );
        assert_eq!(plain_text_of(para), "Press Ctrl+C or code.");
    }

    fn plain_text_of(node: &Node) -> String {
        lsdocs_plugin_sdk::doctree::plain_text(node.children())
    }

    #[test]
    fn directive_options_in_yaml_and_problems() {
        let input = "```{versionadded} 2\n---\nclass: big\nhidden: true\n---\nText.\n```\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let value = serde_json::to_value(&doc.children[0]).unwrap();
        assert_eq!(
            value["attrs"]["options"],
            json!({"class": "big", "hidden": true})
        );
        assert_eq!(value["children"][0]["type"], "paragraph");
        let input = "```{versionadded}\n:nope: 1\n:hidden: yes\n```\n";
        let (_, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        let messages: Vec<_> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "directive \"versionadded\": 1 argument(s) required, 0 supplied",
                "directive \"versionadded\": unknown option: \"nope\"",
                "directive \"versionadded\": option \"hidden\" takes no value",
            ]
        );
        assert!(diagnostics.iter().all(|d| d.span.is_some()));
    }

    #[test]
    fn undeclared_directives_and_roles() {
        let input = "```{mermaid} arg\n:theme: dark\n\ngraph TD; A-->B\n```\n\n{kbd}`x`\n";
        let doc = parse_ok(input);
        let value = serde_json::to_value(&doc.children).unwrap();
        assert_eq!(value[0]["kind"], "mermaid");
        assert_eq!(
            value[0]["attrs"],
            json!({"argument": "arg", "options": {"theme": "dark"}, "content": "graph TD; A-->B"})
        );
        let kinds: Vec<_> = doc.children[1]
            .children()
            .iter()
            .map(|n| n.kind())
            .collect();
        assert_eq!(kinds, ["text", "inline_code"]);
    }

    #[test]
    fn directives_nest_in_containers() {
        let input = "> ```{versionadded} 1\n> Inside **quote**.\n> ```\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let quote = &doc.children[0];
        let Node::Custom(added) = &quote.children()[0] else {
            panic!("{doc:?}")
        };
        let strong = &added.children[0].children()[1];
        assert_eq!(at(input, strong), "**quote**");
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
}
