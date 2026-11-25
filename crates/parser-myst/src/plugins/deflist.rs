//! Definition lists, as pulldown-cmark reads them: a one-line term, then
//! one or more `: definition` lines (later lines of a definition indented
//! at least two spaces, or lazy paragraph continuation lines). A term after blank lines, followed by a
//! definition, continues the list.

use markdown_it::common::sourcemap::SourcePos;
use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::inline::InlineRoot;
use markdown_it::plugins::cmark::block::paragraph::ParagraphScanner;
use markdown_it::{MarkdownIt, Node, NodeValue};

/// A definition list; its children are terms and descriptions.
#[derive(Debug)]
pub(crate) struct DefinitionList;

impl NodeValue for DefinitionList {}

/// A term; its children are inline.
#[derive(Debug)]
pub(crate) struct DefinitionTerm;

impl NodeValue for DefinitionTerm {}

/// A description; its children are blocks.
#[derive(Debug)]
pub(crate) struct DefinitionDescription;

impl NodeValue for DefinitionDescription {}

/// Adds definition lists.
pub(crate) fn add(md: &mut MarkdownIt) {
    md.block
        .add_rule::<DefinitionListScanner>()
        .before::<ParagraphScanner>();
}

struct DefinitionListScanner;

/// Where `line`'s `: ` marker's content starts, if it is a definition.
fn definition(state: &BlockState, line: usize) -> Option<usize> {
    if line >= state.line_max || state.is_empty(line) || state.line_indent(line) >= 4 {
        return None;
    }
    let rest = state.get_line(line).strip_prefix(':')?;
    let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    if spaces == 0 && !rest.is_empty() {
        return None;
    }
    Some(state.line_offsets[line].first_nonspace + 1 + spaces)
}

/// Whether `line` is a term: a non-empty line followed by a definition.
fn term(state: &BlockState, line: usize) -> bool {
    line < state.line_max
        && !state.is_empty(line)
        && state.line_indent(line) < 4
        && definition(state, line).is_none()
        && definition(state, line + 1).is_some()
}

fn line_pos(state: &BlockState, from: usize, to_line: usize) -> SourcePos {
    SourcePos::new(from, state.line_offsets[to_line].line_end)
}

impl BlockRule for DefinitionListScanner {
    fn check(_: &mut BlockState) -> Option<()> {
        None
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let start = state.line;
        if !term(state, start) {
            return None;
        }
        let mut list = Node::new(DefinitionList);
        let mut line = start;
        loop {
            let term_from = state.line_offsets[line].line_start;
            let (content, mapping) = state.get_lines(line, line + 1, state.blk_indent, false);
            let mut node = Node::new(DefinitionTerm);
            node.children
                .push(Node::new(InlineRoot::new(content, mapping)));
            node.srcmap = Some(line_pos(state, term_from, line));
            list.children.push(node);
            line += 1;
            while let Some(content_start) = definition(state, line) {
                let mut end = line + 1;
                while end < state.line_max
                    && !state.is_empty(end)
                    && definition(state, end).is_none()
                    && !term(state, end)
                    && (state.line_indent(end) >= 2 || super::lazy(state, end))
                {
                    end += 1;
                }
                let description = Self::description(state, line, end, content_start);
                list.children.push(description);
                line = end;
            }
            let next = state.skip_empty_lines(line);
            if !term(state, next) {
                break;
            }
            // The last description runs over the blank lines (as
            // pulldown-cmark's does), up to the next term.
            let next_start = state.line_offsets[next].line_start;
            if let Some(pos) = list.children.last_mut().and_then(|d| d.srcmap.as_mut()) {
                *pos = SourcePos::new(pos.get_byte_offsets().0, next_start.saturating_sub(1));
            }
            line = next;
        }
        Some((list, line - start))
    }
}

impl DefinitionListScanner {
    /// The description on lines `start..end`, its content starting at
    /// `content_start`.
    fn description(state: &mut BlockState, start: usize, end: usize, content_start: usize) -> Node {
        let old_node = std::mem::replace(&mut state.node, Node::new(DefinitionDescription));
        let old_offset = state.line_offsets[start].clone();
        let old_indent = state.blk_indent;
        let old_line = state.line;
        let old_line_max = state.line_max;
        state.blk_indent += 2;
        state.line_offsets[start].first_nonspace = content_start;
        state.line_offsets[start].indent_nonspace = state.blk_indent as i32;
        state.line = start;
        state.line_max = end;
        state.md.block.tokenize(state);
        state.line = old_line;
        state.line_max = old_line_max;
        state.blk_indent = old_indent;
        state.line_offsets[start] = old_offset;
        let mut node = std::mem::replace(&mut state.node, old_node);
        node.srcmap = Some(line_pos(
            state,
            state.line_offsets[start].line_start,
            end - 1,
        ));
        node
    }
}
