//! Line endings and spans: markdown-it-rs 0.7 mishandles `\r\n` (source
//! maps, and an overflow in its emphasis rule), so text is parsed with
//! `\n` line endings and spans are mapped back to the file. Spans its
//! source maps get wrong (outside the text, or off a character boundary)
//! are dropped.

use std::borrow::Cow;

use lsdocs_plugin_sdk::doctree::{Document, Node, Span};
use lsdocs_plugin_sdk::protocol::Diagnostic;

/// Text with `\n` line endings, and where `\r`s were taken out.
pub(crate) struct Normalized<'a> {
    /// The text parsed.
    pub(crate) text: Cow<'a, str>,
    /// Offsets in `text` of each `\n` that was a `\r\n`.
    removed: Vec<usize>,
}

/// `input` with `\r\n` and `\r` line endings as `\n`.
pub(crate) fn normalize(input: &str) -> Normalized<'_> {
    if !input.contains('\r') {
        return Normalized {
            text: Cow::Borrowed(input),
            removed: Vec::new(),
        };
    }
    let mut text = String::with_capacity(input.len());
    let mut removed = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                removed.push(text.len());
            } else {
                text.push('\n');
            }
        } else {
            text.push(c);
        }
    }
    Normalized {
        text: Cow::Owned(text),
        removed,
    }
}

impl Normalized<'_> {
    /// The file offset of `offset` in the text.
    fn original(&self, offset: u32) -> u32 {
        let n = offset as usize;
        let before = self.removed.partition_point(|&r| r < n);
        u32::try_from(n + before).unwrap_or(u32::MAX)
    }

    /// `span` (in the text) in the file, or `None` if it is not a span of
    /// the text.
    fn span(&self, span: Span) -> Option<Span> {
        let (start, end) = (span.start as usize, span.end as usize);
        let text = &self.text;
        if start > end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            return None;
        }
        Some(Span::new(
            self.original(span.start),
            self.original(span.end),
        ))
    }

    /// Maps the spans of `document` and `diagnostics` to the file.
    pub(crate) fn map(&self, document: &mut Document, diagnostics: &mut [Diagnostic]) {
        self.map_nodes(&mut document.children);
        for d in diagnostics {
            d.span = d.span.and_then(|s| self.span(s));
        }
    }

    fn map_nodes(&self, nodes: &mut [Node]) {
        for node in nodes {
            let span = node.span_mut();
            *span = span.and_then(|s| self.span(s));
            if let Some(children) = node.children_mut() {
                self.map_nodes(children);
            }
        }
    }
}
