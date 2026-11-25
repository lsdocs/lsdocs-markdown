//! markdown-it's syntax tree to lsdocs' document shape.

use std::collections::BTreeMap;

use lsdocs_plugin_sdk::doctree::{
    Admonition, Align, BlockQuote, CodeBlock, Custom, Document, Emphasis, Heading, Image,
    InlineCode, LineBreak, Link, List, ListItem, Metadata, Node, Paragraph, Raw, Section, Span,
    Strong, Table, TableCell, TableRow, Text, ThematicBreak, Value,
};
use lsdocs_plugin_sdk::protocol::{Diagnostic, Severity};
use markdown_it::Node as MdNode;
use markdown_it::parser::core::Root;
use markdown_it::parser::inline::{Text as MdText, TextSpecial};
use markdown_it::plugins::cmark::block::blockquote::Blockquote;
use markdown_it::plugins::cmark::block::code::CodeBlock as MdCodeBlock;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::plugins::cmark::block::heading::ATXHeading;
use markdown_it::plugins::cmark::block::hr::ThematicBreak as MdThematicBreak;
use markdown_it::plugins::cmark::block::lheading::SetextHeader;
use markdown_it::plugins::cmark::block::list::{BulletList, ListItem as MdListItem, OrderedList};
use markdown_it::plugins::cmark::block::paragraph::Paragraph as MdParagraph;
use markdown_it::plugins::cmark::inline::autolink::Autolink;
use markdown_it::plugins::cmark::inline::backticks::CodeInline;
use markdown_it::plugins::cmark::inline::emphasis::{Em, Strong as MdStrong};
use markdown_it::plugins::cmark::inline::image::Image as MdImage;
use markdown_it::plugins::cmark::inline::link::Link as MdLink;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::plugins::extra::front_matter::{FrontMatter, FrontMatterKind};
use markdown_it::plugins::extra::tables::{
    ColumnAlignment, Table as MdTable, TableBody, TableCell as MdTableCell, TableHead,
    TableRow as MdTableRow,
};
use markdown_it::plugins::extra::tasklist::TaskListMarker;
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::plugins::html::html_inline::HtmlInline;

use crate::plugins::deflist::{DefinitionDescription, DefinitionList, DefinitionTerm};
use crate::plugins::directive::{Directive, DirectiveArgument};
use crate::plugins::footnote::{FootnoteDefinition, FootnoteReference};
use crate::plugins::math::Math;
use crate::plugins::role::Role;

/// The document `root` (parsed from `input`) is, plus any problems found.
pub(crate) fn document(source: &str, input: &str, root: &MdNode) -> (Document, Vec<Diagnostic>) {
    let mut converter = Converter {
        input,
        document: Document::new(source),
        diagnostics: Vec::new(),
        checked: None,
    };
    if let Some(front) = root.cast::<Root>().and_then(|r| r.ext.get::<FrontMatter>()) {
        converter.front_matter(front);
    }
    let children = converter.nodes(&root.children);
    converter.document.children = sectionize(children);
    (converter.document, converter.diagnostics)
}

struct Converter<'a> {
    /// The text parsed.
    input: &'a str,
    document: Document,
    diagnostics: Vec<Diagnostic>,
    /// The task list marker of the list item being converted.
    checked: Option<bool>,
}

impl Converter<'_> {
    /// An inline node's span: its source map.
    fn span(&self, node: &MdNode) -> Option<Span> {
        let (start, end) = node.srcmap?.get_byte_offsets();
        Some(Span::new(clamp(start), clamp(end)))
    }

    /// A block node's span: its source map and the line ending after it,
    /// as pulldown-cmark's are.
    fn block_span(&self, node: &MdNode) -> Option<Span> {
        let (start, end) = node.srcmap?.get_byte_offsets();
        let end = if self.input[end.min(self.input.len())..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        Some(Span::new(clamp(start), clamp(end)))
    }

    /// A table cell's span: its source map and the padding around it, up
    /// to its pipes, as pulldown-cmark's are.
    fn cell_span(&self, node: &MdNode) -> Option<Span> {
        let (start, end) = node.srcmap?.get_byte_offsets();
        let pad = |c: char| c == ' ' || c == '\t';
        let start =
            start - (self.input[..start].len() - self.input[..start].trim_end_matches(pad).len());
        let rest = &self.input[end..];
        let end = end + (rest.len() - rest.trim_start_matches(pad).len());
        Some(Span::new(clamp(start), clamp(end)))
    }

    fn nodes(&mut self, children: &[MdNode]) -> Vec<Node> {
        let mut out = Vec::new();
        for child in children {
            self.node(child, &mut out);
        }
        out
    }

    /// Converts `node`, adding what it becomes to `out`.
    fn node(&mut self, node: &MdNode, out: &mut Vec<Node>) {
        let converted = if let Some(text) = node.cast::<MdText>() {
            text_node(self.span(node), &text.content)
        } else if let Some(text) = node.cast::<TextSpecial>() {
            text_node(self.span(node), &text.content)
        } else if node.is::<Softbreak>() {
            text_node(self.span(node), "\n")
        } else if node.is::<Hardbreak>() {
            Node::LineBreak(LineBreak {
                span: self.span(node),
            })
        } else if node.is::<MdParagraph>() {
            Node::Paragraph(Paragraph {
                span: self.block_span(node),
                children: self.nodes(&node.children),
            })
        } else if let Some(h) = node.cast::<ATXHeading>() {
            self.heading(node, h.level)
        } else if let Some(h) = node.cast::<SetextHeader>() {
            self.heading(node, h.level)
        } else if node.is::<Blockquote>() {
            if let Some(alert) = self.alert(node) {
                alert
            } else {
                Node::BlockQuote(BlockQuote {
                    span: self.block_span(node),
                    children: self.nodes(&node.children),
                })
            }
        } else if node.is::<MdThematicBreak>() {
            Node::ThematicBreak(ThematicBreak {
                span: self.block_span(node),
            })
        } else if let Some(fence) = node.cast::<CodeFence>() {
            let (language, info) = split_info(&fence.info);
            Node::CodeBlock(CodeBlock {
                // Through the closing fence, as pulldown-cmark's span.
                span: self.span(node),
                language,
                info,
                value: fence.content.clone(),
            })
        } else if let Some(code) = node.cast::<MdCodeBlock>() {
            Node::CodeBlock(CodeBlock {
                span: self.block_span(node),
                language: None,
                info: None,
                value: code.content.clone(),
            })
        } else if node.is::<BulletList>() {
            Node::List(List {
                span: self.block_span(node),
                ordered: false,
                start: None,
                children: self.nodes(&node.children),
            })
        } else if let Some(list) = node.cast::<OrderedList>() {
            Node::List(List {
                span: self.block_span(node),
                ordered: true,
                start: Some(u64::from(list.start)).filter(|&n| n != 1),
                children: self.nodes(&node.children),
            })
        } else if node.is::<MdListItem>() {
            let outer = self.checked.take();
            let children = self.nodes(&node.children);
            let checked = std::mem::replace(&mut self.checked, outer);
            Node::ListItem(ListItem {
                span: self.block_span(node),
                checked,
                children,
            })
        } else if let Some(marker) = node.cast::<TaskListMarker>() {
            self.checked = Some(marker.checked);
            return;
        } else if node.is::<Em>() {
            Node::Emphasis(Emphasis {
                span: self.span(node),
                children: self.nodes(&node.children),
            })
        } else if node.is::<MdStrong>() {
            Node::Strong(Strong {
                span: self.span(node),
                children: self.nodes(&node.children),
            })
        } else if node.is::<CodeInline>() {
            Node::InlineCode(InlineCode {
                span: self.span(node),
                value: node.collect_text(),
            })
        } else if let Some(link) = node.cast::<MdLink>() {
            Node::Link(Link {
                span: self.span(node),
                url: link.url.clone(),
                title: link.title.clone().filter(|t| !t.is_empty()),
                target: None,
                children: self.nodes(&node.children),
            })
        } else if let Some(link) = node.cast::<Autolink>() {
            Node::Link(Link {
                span: self.span(node),
                url: link.url.clone(),
                title: None,
                target: None,
                children: self.nodes(&node.children),
            })
        } else if let Some(image) = node.cast::<MdImage>() {
            Node::Image(Image {
                span: self.span(node),
                url: image.url.clone(),
                alt: node.collect_text(),
                title: image.title.clone().filter(|t| !t.is_empty()),
            })
        } else if let Some(html) = node.cast::<HtmlBlock>() {
            Node::Raw(Raw {
                span: self.block_span(node),
                format: "html".into(),
                value: html.content.clone(),
            })
        } else if let Some(html) = node.cast::<HtmlInline>() {
            Node::Raw(Raw {
                span: self.span(node),
                format: "html".into(),
                value: html.content.clone(),
            })
        } else if let Some(r) = node.cast::<FootnoteReference>() {
            Node::FootnoteReference(lsdocs_plugin_sdk::doctree::FootnoteReference {
                span: self.span(node),
                label: r.label.clone(),
            })
        } else if let Some(d) = node.cast::<FootnoteDefinition>() {
            Node::FootnoteDefinition(lsdocs_plugin_sdk::doctree::FootnoteDefinition {
                span: self.block_span(node),
                label: d.label.clone(),
                children: self.nodes(&node.children),
            })
        } else if let Some(m) = node.cast::<Math>() {
            Node::Math(lsdocs_plugin_sdk::doctree::Math {
                span: self.span(node),
                display: m.display,
                value: m.value.clone(),
            })
        } else if node.is::<DefinitionList>() {
            Node::DefinitionList(lsdocs_plugin_sdk::doctree::DefinitionList {
                span: self.block_span(node),
                children: self.nodes(&node.children),
            })
        } else if node.is::<DefinitionTerm>() {
            Node::DefinitionTerm(lsdocs_plugin_sdk::doctree::DefinitionTerm {
                span: self.block_span(node),
                children: self.nodes(&node.children),
            })
        } else if node.is::<DefinitionDescription>() {
            // Tight: a paragraph's inlines are the description's own.
            let mut children = Vec::new();
            for child in &node.children {
                if child.is::<MdParagraph>() {
                    for inline in &child.children {
                        self.node(inline, &mut children);
                    }
                } else {
                    self.node(child, &mut children);
                }
            }
            Node::DefinitionDescription(lsdocs_plugin_sdk::doctree::DefinitionDescription {
                span: self.block_span(node),
                children,
            })
        } else if let Some(d) = node.cast::<Directive>() {
            let span = self.span(node);
            if let Some(error) = &d.options_error {
                let message = format!("directive \"{}\": {error}", d.name);
                self.warn_with(span, message, "directive");
            }
            let mut children = self.nodes(&node.children);
            if let Some(end) = d.body_end {
                clamp_ends(&mut children, clamp(end));
            }
            for problem in &d.problems {
                let message = format!("directive \"{}\": {problem}", d.name);
                self.warn_with(span, message, "directive");
            }
            Node::Custom(Custom {
                span,
                kind: d.name.clone(),
                attrs: d.attrs.clone(),
                children,
            })
        } else if node.is::<DirectiveArgument>() {
            Node::Custom(Custom {
                span: self.span(node),
                kind: "directive_argument".into(),
                attrs: BTreeMap::new(),
                children: self.nodes(&node.children),
            })
        } else if let Some(role) = node.cast::<Role>() {
            let span = self.span(node);
            Node::Custom(Custom {
                span,
                kind: "role".into(),
                attrs: BTreeMap::from([
                    ("name".into(), Value::String(role.name.clone())),
                    ("text".into(), Value::String(role.text.clone())),
                ]),
                children: vec![text_node(span, &role.text)],
            })
        } else if let Some(table) = node.cast::<MdTable>() {
            self.table(node, table)
        } else if node.is::<MdTableRow>() {
            self.row(node, false)
        } else if node.is::<MdTableCell>() {
            Node::TableCell(TableCell {
                span: self.cell_span(node),
                children: self.nodes(&node.children),
            })
        } else {
            // A container without a node of its own: keep its content.
            for child in &node.children {
                self.node(child, out);
            }
            return;
        };
        push(out, converted);
    }

    fn heading(&mut self, node: &MdNode, level: u8) -> Node {
        let mut children = self.nodes(&node.children);
        let id = heading_id(&mut children);
        Node::Heading(Heading {
            span: self.block_span(node),
            level,
            id,
            children,
        })
    }

    /// A GitHub alert: a block quote whose first line is `[!KIND]`, as an
    /// admonition without that line.
    fn alert(&mut self, node: &MdNode) -> Option<Node> {
        let first = node.children.first().filter(|p| p.is::<MdParagraph>())?;
        let mut marker = String::new();
        let mut rest = first.children.len();
        for (i, inline) in first.children.iter().enumerate() {
            if inline.is::<Softbreak>() {
                rest = i + 1;
                break;
            }
            if let Some(text) = inline.cast::<MdText>() {
                marker.push_str(&text.content);
            } else {
                marker.push_str(&inline.cast::<TextSpecial>()?.content);
            }
        }
        let kind = marker
            .trim_end()
            .strip_prefix("[!")?
            .strip_suffix(']')?
            .to_lowercase();
        let kind = ["note", "tip", "important", "warning", "caution"]
            .into_iter()
            .find(|k| *k == kind)?;
        let mut children = Vec::new();
        let inlines = self.nodes(&first.children[rest..]);
        if let Some(start) = inlines.first().and_then(Node::span) {
            let span = self
                .block_span(first)
                .map(|s| Span::new(start.start, s.end));
            children.push(Node::Paragraph(Paragraph {
                span,
                children: inlines,
            }));
        }
        for child in &node.children[1..] {
            self.node(child, &mut children);
        }
        Some(Node::Admonition(Admonition {
            span: self.block_span(node),
            kind: kind.into(),
            title: None,
            children,
        }))
    }

    fn table(&mut self, node: &MdNode, table: &MdTable) -> Node {
        let mut children = Vec::new();
        for part in &node.children {
            let header = part.is::<TableHead>();
            if header || part.is::<TableBody>() {
                for row in &part.children {
                    children.push(self.row(row, header));
                }
            } else {
                self.node(part, &mut children);
            }
        }
        Node::Table(Table {
            span: self.block_span(node),
            align: table
                .alignments
                .iter()
                .map(|a| match a {
                    ColumnAlignment::None => Align::None,
                    ColumnAlignment::Left => Align::Left,
                    ColumnAlignment::Center => Align::Center,
                    ColumnAlignment::Right => Align::Right,
                })
                .collect(),
            children,
        })
    }

    fn row(&mut self, node: &MdNode, header: bool) -> Node {
        Node::TableRow(TableRow {
            span: self.block_span(node),
            header,
            children: self.nodes(&node.children),
        })
    }

    fn front_matter(&mut self, front: &FrontMatter) {
        let start = line_offset(self.input, front.start_line);
        // Through the closing fence, not its line ending (as
        // pulldown-cmark's span).
        let end = line_offset(self.input, front.end_line + 1);
        let end = if self.input[..end].ends_with('\n') {
            end - 1
        } else {
            end
        };
        let span = Span::new(clamp(start), clamp(end));
        let (parsed, format): (Result<Value, String>, _) = match front.kind {
            FrontMatterKind::Yaml => (
                serde_saphyr::from_str(&front.raw).map_err(|e| e.to_string()),
                "YAML",
            ),
            FrontMatterKind::Toml => (
                toml::from_str(&front.raw).map_err(|e| e.to_string()),
                "TOML",
            ),
        };
        match parsed {
            Ok(Value::Object(map)) => {
                self.document.metadata = map.into_iter().collect::<Metadata>();
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
}

fn clamp(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Where line `line` (0-based) of `input` starts (its length past the end).
fn line_offset(input: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    input
        .match_indices('\n')
        .nth(line - 1)
        .map_or(input.len(), |(i, _)| i + 1)
}

/// Ends the spans of `nodes` and their descendants at `end` at the latest.
fn clamp_ends(nodes: &mut [Node], end: u32) {
    for node in nodes {
        if let Some(span) = node.span_mut() {
            span.end = span.end.min(end);
        }
        if let Some(children) = node.children_mut() {
            clamp_ends(children, end);
        }
    }
}

fn text_node(span: Option<Span>, value: &str) -> Node {
    Node::Text(Text {
        span,
        value: value.to_owned(),
    })
}

/// Takes a trailing `{#id ...}` off a heading's text: the id, if any.
/// Other attributes (`.class`, `key=value`) are dropped.
fn heading_id(children: &mut Vec<Node>) -> Option<String> {
    let Some(Node::Text(text)) = children.last_mut() else {
        return None;
    };
    let trimmed = text.value.trim_end();
    let open = trimmed.strip_suffix('}')?.rfind('{')?;
    let attrs = &trimmed[open + 1..trimmed.len() - 1];
    let mut id = None;
    for attr in attrs.split_whitespace() {
        if let Some(value) = attr.strip_prefix('#') {
            id = Some(value.to_owned());
        } else if !attr.starts_with('.') && !attr.contains('=') {
            return None;
        }
    }
    let kept = text.value[..open].trim_end().len();
    let removed = text.value.len() - kept;
    text.value.truncate(kept);
    if let Some(span) = &mut text.span {
        span.end = span.end.saturating_sub(u32::try_from(removed).unwrap_or(0));
    }
    if text.value.is_empty() {
        children.pop();
    }
    id
}

/// A fence's info string as language and the rest.
fn split_info(info: &str) -> (Option<String>, Option<String>) {
    let info = info.trim();
    match info.split_once(char::is_whitespace) {
        Some((lang, rest)) => (Some(lang.to_owned()), Some(rest.trim().to_owned())),
        None if info.is_empty() => (None, None),
        None => (Some(info.to_owned()), None),
    }
}

/// Adds `node` to `out`, merging adjacent text.
fn push(out: &mut Vec<Node>, node: Node) {
    if let (Node::Text(new), Some(Node::Text(prev))) = (&node, out.last_mut()) {
        prev.value.push_str(&new.value);
        if let (Some(a), Some(b)) = (prev.span, new.span) {
            prev.span = Some(Span::new(a.start.min(b.start), a.end.max(b.end)));
        }
        return;
    }
    out.push(node);
}

/// Nests top-level nodes into sections: each heading opens a section that
/// holds everything up to the next heading of the same or a higher level
/// (a lower or equal level number). The heading's id moves to the section.
pub(crate) fn sectionize(nodes: Vec<Node>) -> Vec<Node> {
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
