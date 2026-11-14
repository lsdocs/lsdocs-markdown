//! First-party Markdown parser plugin.
//!
//! This is a deliberately small parser covering block structure only:
//! front matter (`key: value` lines between `---` fences), ATX headings,
//! fenced code blocks and paragraphs. Inline markup is kept as plain text.
//! Phase 4 replaces it with a full CommonMark parser.

use std::collections::BTreeMap;

use lsdocs_plugin_sdk::doctree::{
    CodeBlock, Document, Heading, Node, Paragraph, Span, Value, text,
};
use lsdocs_plugin_sdk::protocol::{ParseParams, ParseResult};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};

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
        Ok(ParseResult {
            document: parse(&params.source, &params.text),
            diagnostics: vec![],
        })
    }
}

/// Parses Markdown text into a document.
pub fn parse(source: &str, input: &str) -> Document {
    let mut doc = Document::new(source);
    let mut offset = 0;
    let body = match front_matter(input) {
        Some((metadata, rest_at)) => {
            doc.metadata = metadata;
            offset = rest_at;
            &input[rest_at..]
        }
        None => input,
    };

    let mut lines = Lines::new(body, offset);
    while let Some((start, line)) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(fence) = line.trim_start().strip_prefix("```") {
            let language = Some(fence.trim().to_owned()).filter(|l| !l.is_empty());
            let mut code = String::new();
            let mut end = start + line.len();
            for (at, line) in lines.by_ref() {
                end = at + line.len();
                if line.trim_start().starts_with("```") {
                    break;
                }
                code.push_str(line);
                code.push('\n');
            }
            doc.children.push(Node::CodeBlock(CodeBlock {
                span: Some(span(start, end)),
                language,
                info: None,
                value: code,
            }));
            continue;
        }
        if let Some((level, title)) = heading(line) {
            doc.children.push(Node::Heading(Heading {
                span: Some(span(start, start + line.len())),
                level,
                id: Some(slug(title)),
                children: vec![text(title)],
            }));
            continue;
        }
        // A paragraph runs until a blank line, heading or fence.
        let mut words = vec![line.trim()];
        let mut end = start + line.len();
        while let Some((at, next)) = lines.peek() {
            if next.trim().is_empty()
                || heading(next).is_some()
                || next.trim_start().starts_with("```")
            {
                break;
            }
            words.push(next.trim());
            end = at + next.len();
            lines.next();
        }
        doc.children.push(Node::Paragraph(Paragraph {
            span: Some(span(start, end)),
            children: vec![text(words.join(" "))],
        }));
    }
    doc
}

/// Splits leading `---` front matter into metadata and the offset where the
/// body starts.
fn front_matter(text: &str) -> Option<(BTreeMap<String, Value>, usize)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let header_len = text.len() - rest.len();
    let mut metadata = BTreeMap::new();
    let mut consumed = 0;
    for line in rest.split_inclusive('\n') {
        consumed += line.len();
        let trimmed = line.trim_end();
        if trimmed == "---" {
            return Some((metadata, header_len + consumed));
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim().trim_matches('"');
            metadata.insert(key.trim().to_owned(), Value::String(value.to_owned()));
        }
    }
    None
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

/// A URL-friendly identifier for a heading.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn span(start: usize, end: usize) -> Span {
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Span::new(clamp(start), clamp(end))
}

/// Lines with their byte offsets, excluding line endings.
struct Lines<'a> {
    rest: &'a str,
    at: usize,
    peeked: Option<(usize, &'a str)>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, at: usize) -> Self {
        Self {
            rest: text,
            at,
            peeked: None,
        }
    }

    fn peek(&mut self) -> Option<(usize, &'a str)> {
        if self.peeked.is_none() {
            self.peeked = self.advance();
        }
        self.peeked
    }

    fn advance(&mut self) -> Option<(usize, &'a str)> {
        if self.rest.is_empty() {
            return None;
        }
        let (line, len) = match self.rest.find('\n') {
            Some(i) => (&self.rest[..i], i + 1),
            None => (self.rest, self.rest.len()),
        };
        let start = self.at;
        self.rest = &self.rest[len..];
        self.at += len;
        Some((start, line.strip_suffix('\r').unwrap_or(line)))
    }
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        self.peeked.take().or_else(|| self.advance())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks_with_spans() {
        let text = "---\ntitle: \"Hello\"\n---\n# Hello World\n\nSome\ntext.\n\n```rust\nfn x() {}\n```\n## Next\n";
        let doc = parse("a.md", text);
        assert_eq!(doc.metadata["title"], Value::String("Hello".into()));
        let kinds: Vec<_> = doc.children.iter().map(|n| n.kind()).collect();
        assert_eq!(kinds, ["heading", "paragraph", "code_block", "heading"]);
        let Node::Heading(h) = &doc.children[0] else {
            panic!()
        };
        assert_eq!((h.level, h.id.as_deref()), (1, Some("hello-world")));
        let span = h.span.unwrap();
        assert_eq!(
            &text[span.start as usize..span.end as usize],
            "# Hello World"
        );
        let Node::Paragraph(p) = &doc.children[1] else {
            panic!()
        };
        assert_eq!(p.children, [lsdocs_plugin_sdk::doctree::text("Some text.")]);
        let Node::CodeBlock(c) = &doc.children[2] else {
            panic!()
        };
        assert_eq!(
            (c.language.as_deref(), c.value.as_str()),
            (Some("rust"), "fn x() {}\n")
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Getting Started!"), "getting-started");
        assert_eq!(slug("  Ünïcode & more  "), "ünïcode-more");
    }

    #[test]
    fn not_headings() {
        assert!(heading("#hashtag").is_none());
        assert!(heading("####### seven").is_none());
        assert_eq!(heading("### Three ###"), Some((3, "Three")));
    }
}
