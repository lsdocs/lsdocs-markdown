//! The markdown-it parser: CommonMark and the plugins MyST needs.

use lsdocs_plugin_sdk::protocol::Syntax;
use markdown_it::MarkdownIt;
use markdown_it::plugins::{cmark, extra, html};

use crate::plugins::{deflist, directive, footnote, math, role, target};

/// A parser for `syntax`'s directives and roles.
pub(crate) fn parser(syntax: &Syntax) -> MarkdownIt {
    let mut md = MarkdownIt::new();
    cmark::add(&mut md);
    html::add(&mut md);
    extra::tables::add(&mut md);
    extra::tasklist::add(&mut md);
    extra::front_matter::add(&mut md);
    footnote::add(&mut md);
    math::add(&mut md);
    deflist::add(&mut md);
    directive::add(&mut md, syntax);
    role::add(&mut md);
    target::add(&mut md);
    md
}
