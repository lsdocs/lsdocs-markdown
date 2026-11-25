//! Footnotes, as pulldown-cmark reads them: `[^label]` references to a
//! label defined in the document, and `[^label]: ...` definitions, whose
//! later blocks are indented four spaces. Definitions stay where they are
//! written.

use std::collections::HashSet;

use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::extset::RootExt;
use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::plugins::cmark::block::reference::ReferenceScanner;
use markdown_it::{MarkdownIt, Node, NodeValue};

/// A footnote reference.
#[derive(Debug)]
pub(crate) struct FootnoteReference {
    /// The label, as written.
    pub(crate) label: String,
}

impl NodeValue for FootnoteReference {}

/// A footnote definition; its children are its blocks.
#[derive(Debug)]
pub(crate) struct FootnoteDefinition {
    /// The label, as written.
    pub(crate) label: String,
}

impl NodeValue for FootnoteDefinition {}

/// The labels defined in the document (lowercase).
#[derive(Debug, Default)]
struct Defined(HashSet<String>);

impl RootExt for Defined {}

/// Indent of a definition's later lines.
const INDENT: i32 = 4;

/// Adds footnote references and definitions.
pub(crate) fn add(md: &mut MarkdownIt) {
    md.block
        .add_rule::<DefinitionScanner>()
        .before::<ReferenceScanner>();
    md.inline.add_rule::<ReferenceInline>().before_named("link");
}

/// `[^label]` at the start of `text`: the label and the length.
fn label(text: &str) -> Option<(&str, usize)> {
    let rest = text.strip_prefix("[^")?;
    let end = rest.find(']')?;
    let label = &rest[..end];
    if label.is_empty() || label.contains(char::is_whitespace) {
        return None;
    }
    Some((label, end + 3))
}

struct DefinitionScanner;

impl BlockRule for DefinitionScanner {
    fn check(state: &mut BlockState) -> Option<()> {
        Self::scan(state).map(|_| ())
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let (label, content_start) = Self::scan(state)?;
        state
            .root_ext
            .get_or_insert_default::<Defined>()
            .0
            .insert(label.to_lowercase());
        let start_line = state.line;
        let mut end_line = start_line + 1;
        let mut line = end_line;
        while line < state.line_max {
            if !state.is_empty(line) {
                if state.line_indent(line) < INDENT {
                    break;
                }
                end_line = line + 1;
            }
            line += 1;
        }
        let old_node = std::mem::replace(&mut state.node, Node::new(FootnoteDefinition { label }));
        let old_offset = state.line_offsets[start_line].clone();
        let old_indent = state.blk_indent;
        let old_line_max = state.line_max;
        state.blk_indent += INDENT as usize;
        state.line_offsets[start_line].first_nonspace = content_start;
        state.line_offsets[start_line].indent_nonspace = state.blk_indent as i32;
        state.line_max = end_line;
        state.md.block.tokenize(state);
        let next_line = state.line;
        state.line = start_line;
        state.line_max = old_line_max;
        state.blk_indent = old_indent;
        state.line_offsets[start_line] = old_offset;
        let node = std::mem::replace(&mut state.node, old_node);
        Some((node, next_line - start_line))
    }
}

impl DefinitionScanner {
    /// The label of a definition starting on the current line, and where
    /// its content starts in the source.
    fn scan(state: &BlockState) -> Option<(String, usize)> {
        if state.line_indent(state.line) >= INDENT {
            return None;
        }
        let line = state.get_line(state.line);
        let (label, len) = label(line)?;
        let rest = line[len..].strip_prefix(':')?;
        let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let offsets = &state.line_offsets[state.line];
        Some((label.to_owned(), offsets.first_nonspace + len + 1 + spaces))
    }
}

struct ReferenceInline;

impl InlineRule for ReferenceInline {
    const MARKER: char = '[';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let (label, len) = label(&state.src[state.pos..state.pos_max])?;
        let defined = state.root_ext.get::<Defined>()?;
        if !defined.0.contains(&label.to_lowercase()) {
            return None;
        }
        let label = label.to_owned();
        Some((Node::new(FootnoteReference { label }), len))
    }
}
