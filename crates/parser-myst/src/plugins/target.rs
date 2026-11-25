//! MyST targets: a `(label)=` line. It can end a paragraph.

use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::plugins::cmark::block::paragraph::ParagraphScanner;
use markdown_it::{MarkdownIt, Node, NodeValue};

/// A target.
#[derive(Debug)]
pub(crate) struct Target {
    /// The label, as written.
    pub(crate) label: String,
}

impl NodeValue for Target {}

/// Adds targets.
pub(crate) fn add(md: &mut MarkdownIt) {
    md.block
        .add_rule::<TargetScanner>()
        .before::<ParagraphScanner>();
}

struct TargetScanner;

impl TargetScanner {
    fn label(state: &BlockState) -> Option<String> {
        if state.line_indent(state.line) >= state.md.max_indent {
            return None;
        }
        let line = state.get_line(state.line).trim_end();
        let label = line.strip_prefix('(')?.strip_suffix(")=")?;
        if label.is_empty() || label.contains([')', '(']) || label.contains(char::is_whitespace) {
            return None;
        }
        Some(label.to_owned())
    }
}

impl BlockRule for TargetScanner {
    fn check(state: &mut BlockState) -> Option<()> {
        Self::label(state).map(|_| ())
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let label = Self::label(state)?;
        Some((Node::new(Target { label }), 1))
    }
}
