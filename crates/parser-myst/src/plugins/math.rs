//! Math, as pulldown-cmark reads it: `$x$` inline and `$$x$$` display,
//! both inline. A `$` opener is not followed by whitespace and a `$`
//! closer is not preceded by it.

use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::{MarkdownIt, Node, NodeValue};

/// Math.
#[derive(Debug)]
pub(crate) struct Math {
    /// Display math (`$$`), not inline.
    pub(crate) display: bool,
    /// The TeX source.
    pub(crate) value: String,
}

impl NodeValue for Math {}

/// Adds math.
pub(crate) fn add(md: &mut MarkdownIt) {
    md.inline.add_rule::<MathInline>();
}

struct MathInline;

impl InlineRule for MathInline {
    const MARKER: char = '$';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let text = &state.src[state.pos..state.pos_max];
        if let Some(rest) = text.strip_prefix("$$") {
            let end = rest.find("$$")?;
            let value = rest[..end].to_owned();
            return Some((
                Node::new(Math {
                    display: true,
                    value,
                }),
                end + 4,
            ));
        }
        let rest = text.strip_prefix('$')?;
        if rest.starts_with(char::is_whitespace) || rest.starts_with('$') {
            return None;
        }
        let mut from = 0;
        let end = loop {
            let at = from + rest[from..].find('$')?;
            let before = rest[..at].chars().next_back();
            if at > 0 && !before.is_some_and(|c| c.is_whitespace() || c == '\\') {
                break at;
            }
            from = at + 1;
        };
        let value = rest[..end].to_owned();
        Some((
            Node::new(Math {
                display: false,
                value,
            }),
            end + 2,
        ))
    }
}
