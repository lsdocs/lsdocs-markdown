//! Roles in MyST's syntax: `` {name}`text` `` for a declared role.

use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::{MarkdownIt, Node, NodeValue};

use super::directive::{Declared, is_name};

/// A role.
#[derive(Debug)]
pub(crate) struct Role {
    /// The name, lowercase.
    pub(crate) name: String,
    /// Its text, read as a code span's.
    pub(crate) text: String,
}

impl NodeValue for Role {}

/// Adds roles (the declared ones are in [`Declared`]).
pub(crate) fn add(md: &mut MarkdownIt) {
    md.inline.add_rule::<RoleScanner>();
}

struct RoleScanner;

impl InlineRule for RoleScanner {
    const MARKER: char = '{';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let text = &state.src[state.pos..state.pos_max];
        let rest = text.strip_prefix('{')?;
        let close = rest.find('}')?;
        let name = &rest[..close];
        if !is_name(name) {
            return None;
        }
        let name = name.to_lowercase();
        let declared = state.md.ext.get::<Declared>()?;
        if !declared.0.has_role(&name) {
            return None;
        }
        let after = &rest[close + 1..];
        let ticks = after.chars().take_while(|&c| c == '`').count();
        if ticks == 0 {
            return None;
        }
        // The closing run: exactly as many backticks.
        let code = &after[ticks..];
        let mut from = 0;
        let end = loop {
            let at = from + code[from..].find('`')?;
            let run = code[at..].chars().take_while(|&c| c == '`').count();
            if run == ticks {
                break at;
            }
            from = at + run;
        };
        let text = code_text(&code[..end]);
        let len = 1 + close + 1 + ticks + end + ticks;
        Some((Node::new(Role { name, text }), len))
    }
}

/// A code span's text: line endings as spaces, and one space stripped
/// from each end when both have one (and it is not all spaces).
fn code_text(raw: &str) -> String {
    let text = raw.replace('\n', " ");
    if text.len() > 2 && text.starts_with(' ') && text.ends_with(' ') && text.trim() != "" {
        text[1..text.len() - 1].to_owned()
    } else {
        text
    }
}
