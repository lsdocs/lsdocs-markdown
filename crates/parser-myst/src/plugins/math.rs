//! Math, as pulldown-cmark reads it: `$x$` inline and `$$x$$` display,
//! both inline. A `$` opener is not followed by whitespace and a `$`
//! closer is not preceded by it.

use markdown_it::parser::extset::InlineRootExt;
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

/// From where on (for an end) no closer follows in this inline text:
/// for `$`, and for `$$`. A `$` closer does not depend on its opener, so
/// once an opener finds none, no later one does; this keeps unclosed `$`s
/// linear.
#[derive(Debug, Default)]
struct NoCloser {
    single: Option<(usize, usize)>,
    double: Option<(usize, usize)>,
}

impl InlineRootExt for NoCloser {}

fn closed_out(found: Option<(usize, usize)>, state: &InlineState) -> bool {
    found.is_some_and(|(from, max)| state.pos >= from && state.pos_max == max)
}

struct MathInline;

impl InlineRule for MathInline {
    const MARKER: char = '$';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let cache = state.inline_ext.get::<NoCloser>();
        let (single, double) = cache.map_or((None, None), |c| (c.single, c.double));
        let here = (state.pos, state.pos_max);
        let text = &state.src[state.pos..state.pos_max];
        if let Some(rest) = text.strip_prefix("$$") {
            if closed_out(double, state) {
                return None;
            }
            let Some(end) = rest.find("$$") else {
                state.inline_ext.get_or_insert_default::<NoCloser>().double = Some(here);
                return None;
            };
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
        if closed_out(single, state) {
            return None;
        }
        let mut from = 0;
        let end = loop {
            let Some(found) = rest[from..].find('$') else {
                state.inline_ext.get_or_insert_default::<NoCloser>().single = Some(here);
                return None;
            };
            let at = from + found;
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
