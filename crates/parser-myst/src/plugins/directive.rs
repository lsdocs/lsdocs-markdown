//! Directives in MyST's syntax: a fence of three or more backticks,
//! tildes or colons whose info string starts with `{name}`. The rest of
//! the info line is the argument; leading `:key: value` lines, or a YAML
//! mapping between `---` lines, are options; the rest is content. A
//! fence closes on a line of at least as many of its marker, or at the
//! end of its container.
//!
//! Declared directives (see "Syntax extensions" in `spec/wire.md`) are
//! read by their declaration: options checked, block content parsed in
//! place (so it shares the page's link references and footnotes), inline
//! arguments parsed. Others keep their content as text.

use std::collections::BTreeMap;

use lsdocs_plugin_sdk::doctree::Value;
use lsdocs_plugin_sdk::protocol::{
    ArgumentSyntax, ContentSyntax, DirectiveSyntax, OptionSyntax, Syntax,
};
use markdown_it::common::sourcemap::SourcePos;
use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::extset::MarkdownItExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::plugins::cmark::block::fence::FenceScanner;
use markdown_it::{MarkdownIt, Node, NodeValue};

/// The directives and roles plugins declared.
#[derive(Debug)]
pub(crate) struct Declared(pub(crate) Syntax);

impl MarkdownItExt for Declared {}

/// A directive; its children are its parsed argument and content.
#[derive(Debug)]
pub(crate) struct Directive {
    /// The name, lowercase.
    pub(crate) name: String,
    /// The custom node's attributes: argument, options, text content.
    pub(crate) attrs: BTreeMap<String, Value>,
    /// A problem reading the options, reported first.
    pub(crate) options_error: Option<String>,
    /// Problems against the declaration, reported after the content's.
    pub(crate) problems: Vec<String>,
    /// Where parsed content ends in the source (its last non-blank line's
    /// end): its blocks' spans stop there, as today's parser's did.
    pub(crate) body_end: Option<usize>,
}

impl NodeValue for Directive {}

/// A declared inline argument; its children are inline.
#[derive(Debug)]
pub(crate) struct DirectiveArgument;

impl NodeValue for DirectiveArgument {}

/// Adds directives for `syntax`.
pub(crate) fn add(md: &mut MarkdownIt, syntax: &Syntax) {
    md.ext.insert(Declared(syntax.clone()));
    md.block
        .add_rule::<DirectiveScanner>()
        .before::<FenceScanner>();
}

/// A directive's opening line.
struct Opening {
    marker: char,
    len: usize,
    name: String,
    argument: String,
}

struct DirectiveScanner;

impl DirectiveScanner {
    fn opening(state: &BlockState) -> Option<Opening> {
        if state.line_indent(state.line) >= state.md.max_indent {
            return None;
        }
        let line = state.get_line(state.line);
        let marker = line
            .chars()
            .next()
            .filter(|c| matches!(c, '`' | '~' | ':'))?;
        let len = line.chars().take_while(|&c| c == marker).count();
        if len < 3 {
            return None;
        }
        let rest = line[len..].trim_start().strip_prefix('{')?;
        let close = rest.find('}')?;
        let name = &rest[..close];
        let after = &rest[close + 1..];
        if !is_name(name) || !(after.is_empty() || after.starts_with(char::is_whitespace)) {
            return None;
        }
        Some(Opening {
            marker,
            len,
            name: name.to_lowercase(),
            argument: after.trim().to_owned(),
        })
    }

    /// The line closing the fence opened on the current line, or where
    /// its container ends.
    fn end(state: &BlockState, open: &Opening) -> (usize, bool) {
        let mut next = state.line;
        loop {
            next += 1;
            if next >= state.line_max {
                return (next, false);
            }
            let line = state.get_line(next);
            if !line.is_empty() && state.line_indent(next) < 0 {
                return (next, false);
            }
            if state.line_indent(next) >= state.md.max_indent {
                continue;
            }
            let run = line.chars().take_while(|&c| c == open.marker).count();
            if run >= open.len && line[run..].trim().is_empty() {
                return (next, true);
            }
        }
    }
}

impl BlockRule for DirectiveScanner {
    fn check(state: &mut BlockState) -> Option<()> {
        Self::opening(state).map(|_| ())
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let open = Self::opening(state)?;
        let start = state.line;
        let (end, closed) = Self::end(state, &open);
        let indent = state.line_offsets[start].indent_nonspace.max(0) as usize;
        let (content, _) = state.get_lines(start + 1, end, indent, true);
        let (options, body_start, options_error) = directive_options(&content);
        let body = content[body_start..].trim_end_matches('\n');
        let mut directive = Directive {
            name: open.name.clone(),
            attrs: BTreeMap::new(),
            options_error,
            problems: Vec::new(),
            body_end: None,
        };
        if !open.argument.is_empty() {
            directive
                .attrs
                .insert("argument".into(), Value::String(open.argument.clone()));
        }
        let declared = state
            .md
            .ext
            .get::<Declared>()
            .and_then(|d| d.0.directive(&open.name))
            .cloned();
        let Some(declared) = declared else {
            // Not declared: kept as text, as reStructuredText keeps
            // unknown directives.
            let options: serde_json::Map<String, Value> = options
                .into_iter()
                .map(|(k, v)| (k, Value::String(v.unwrap_or_default())))
                .collect();
            if !options.is_empty() {
                directive
                    .attrs
                    .insert("options".into(), Value::Object(options));
            }
            if !body.is_empty() {
                directive
                    .attrs
                    .insert("content".into(), Value::String(body.to_owned()));
            }
            return Some((Node::new(directive), end - start + usize::from(closed)));
        };
        check_arguments(&declared, &open.argument, &mut directive.problems);
        let values = check_options(&declared, options, &mut directive.problems);
        if !values.is_empty() {
            directive
                .attrs
                .insert("options".into(), Value::Object(values));
        }
        let mut node = Node::new(directive);
        match declared.content {
            ContentSyntax::None if !body.trim().is_empty() => {
                problem(&mut node, "no content permitted");
            }
            ContentSyntax::None => {}
            ContentSyntax::Text => {
                if !body.is_empty()
                    && let Some(d) = node.cast_mut::<Directive>()
                {
                    d.attrs
                        .insert("content".into(), Value::String(body.to_owned()));
                }
            }
            ContentSyntax::Blocks => {
                let body_line = start + 1 + content[..body_start].matches('\n').count();
                let mut last = end;
                while last > body_line && state.is_empty(last - 1) {
                    last -= 1;
                }
                if last > body_line
                    && let Some(d) = node.cast_mut::<Directive>()
                {
                    d.body_end = Some(state.line_offsets[last - 1].line_end);
                }
                node = tokenize(state, node, body_line, end);
            }
        }
        if declared.argument == ArgumentSyntax::Inline && !open.argument.is_empty() {
            let line_start = state.line_offsets[start].first_nonspace;
            let line = state.get_line(start);
            // After `{name}`, not inside it.
            let after = line.find('}').map_or(0, |i| i + 1);
            let at = line_start + after + line[after..].find(open.argument.as_str()).unwrap_or(0);
            let mut argument = Node::new(DirectiveArgument);
            argument.children.push(Node::new(InlineRoot::new(
                open.argument.clone(),
                vec![(0, at)],
            )));
            argument.srcmap = Some(SourcePos::new(at, at + open.argument.len()));
            node.children.insert(0, argument);
        }
        Some((node, end - start + usize::from(closed)))
    }
}

fn problem(node: &mut Node, message: &str) {
    if let Some(d) = node.cast_mut::<Directive>() {
        d.problems.push(message.to_owned());
    }
}

/// Parses lines `from..to` as blocks into `node`, in place.
fn tokenize(state: &mut BlockState, node: Node, from: usize, to: usize) -> Node {
    let old_node = std::mem::replace(&mut state.node, node);
    let old_line = state.line;
    let old_line_max = state.line_max;
    state.line = from;
    state.line_max = to;
    state.md.block.tokenize(state);
    state.line = old_line;
    state.line_max = old_line_max;
    std::mem::replace(&mut state.node, old_node)
}

/// The options' values against `declared`, noting problems.
fn check_options(
    declared: &DirectiveSyntax,
    options: OptionList,
    problems: &mut Vec<String>,
) -> serde_json::Map<String, Value> {
    let mut values = serde_json::Map::new();
    for (key, value) in options {
        match (declared.options.get(&key), value) {
            (None, _) => problems.push(format!("unknown option: \"{key}\"")),
            (Some(OptionSyntax::Flag), Some(v)) if !v.is_empty() => {
                problems.push(format!("option \"{key}\" takes no value"))
            }
            (Some(OptionSyntax::Flag), _) => {
                values.insert(key, Value::Bool(true));
            }
            (Some(OptionSyntax::Required), None) => {
                problems.push(format!("option \"{key}\" needs a value"))
            }
            (Some(OptionSyntax::Required), Some(v)) if v.is_empty() => {
                problems.push(format!("option \"{key}\" needs a value"))
            }
            (Some(_), v) => {
                values.insert(key, Value::String(v.unwrap_or_default()));
            }
        }
    }
    values
}

/// Whether `name` can be a directive or role name: letters, digits and
/// `-`, `_`, `:`, `.`, `+`, starting with a letter or digit.
pub(crate) fn is_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_alphanumeric)
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '+'))
}

/// Directive options as written: names and values (`None` for none).
pub(crate) type OptionList = Vec<(String, Option<String>)>;

/// A directive's options at the start of its fence's text: `:name: value`
/// lines, or a YAML mapping between `---` lines. Returns them (`None` for
/// no value), where the content starts (after one blank line), and a
/// problem with them, if any.
pub(crate) fn directive_options(text: &str) -> (OptionList, usize, Option<String>) {
    let mut options = Vec::new();
    let mut pos = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    let mut error = None;
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        let first = lines.next().unwrap_or_default();
        let mut yaml = String::new();
        let mut end = None;
        let mut at = first.len();
        for line in lines.by_ref() {
            at += line.len();
            if line.trim_end() == "---" {
                end = Some(at);
                break;
            }
            yaml.push_str(line);
        }
        match end {
            Some(end) => {
                pos = end;
                match serde_saphyr::from_str::<Value>(&yaml) {
                    Ok(Value::Object(map)) => {
                        for (key, value) in map {
                            let value = match value {
                                Value::Null | Value::Bool(true) => None,
                                Value::String(s) => Some(s),
                                other => Some(other.to_string()),
                            };
                            options.push((key, value));
                        }
                    }
                    Ok(Value::Null) => {}
                    Ok(_) => error = Some("options must be a mapping".to_owned()),
                    Err(e) => error = Some(format!("invalid options: {e}")),
                }
            }
            None => return (options, 0, None),
        }
    } else {
        for line in lines.by_ref() {
            let Some(rest) = line.trim_end().strip_prefix(':') else {
                break;
            };
            let Some((key, value)) = rest.split_once(':') else {
                break;
            };
            if key.is_empty() || key.contains(char::is_whitespace) {
                break;
            }
            let value = value.trim();
            options.push((
                key.to_owned(),
                (!value.is_empty()).then(|| value.to_owned()),
            ));
            pos += line.len();
        }
    }
    // One blank line may separate options from content.
    if !options.is_empty()
        && let Some(line) = text[pos..].split_inclusive('\n').next()
        && line.trim().is_empty()
    {
        pos += line.len();
    }
    (options, pos, error)
}

/// Problems with a directive's argument against its declaration.
pub(crate) fn check_arguments(
    declared: &DirectiveSyntax,
    argument: &str,
    problems: &mut Vec<String>,
) {
    let most = declared.required_arguments + declared.optional_arguments;
    let words = argument.split_whitespace().count();
    if words < declared.required_arguments as usize {
        problems.push(format!(
            "{} argument(s) required, {words} supplied",
            declared.required_arguments
        ));
    } else if most == 0 && words > 0 {
        problems.push("no arguments permitted".to_owned());
    } else if !declared.final_argument_whitespace && words > most as usize {
        problems.push(format!(
            "maximum {most} argument(s) allowed, {words} supplied"
        ));
    }
}
