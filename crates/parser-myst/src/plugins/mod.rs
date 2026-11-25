//! The markdown-it rules MyST needs beyond CommonMark, each with node
//! types of its own for the converter to read.

pub(crate) mod deflist;
pub(crate) mod directive;
pub(crate) mod footnote;
pub(crate) mod math;
pub(crate) mod role;
pub(crate) mod target;

use markdown_it::parser::block::BlockState;

/// Whether `line` lazily continues the paragraph on the line before it: it
/// is not blank, follows a line that is not, and starts no other block.
pub(crate) fn lazy(state: &mut BlockState, line: usize) -> bool {
    if line == 0 || line >= state.line_max || state.is_empty(line) || state.is_empty(line - 1) {
        return false;
    }
    let old = state.line;
    state.line = line;
    let starts_block = state.test_rules_at_line();
    state.line = old;
    !starts_block
}
