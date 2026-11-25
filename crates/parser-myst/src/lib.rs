//! The MyST Markdown parser plugin, `parser-myst`.
//!
//! Parses Markdown with markdown-it (the `markdown-it-rs` crate), the
//! parser MyST is built on, with MyST's syntax as markdown-it rules, into
//! a document tree in the shared shape (see "Document shape" in
//! `spec/wire.md`). Every node carries the byte span it came from.
//!
//! - Top-level headings open sections: everything up to the next heading of
//!   the same or a higher level is nested in a `section` whose first child
//!   is the heading. Headings inside block quotes or lists stay headings.
//! - Extensions: GitHub tables, task lists and alerts (`> [!NOTE]` becomes
//!   an `admonition`), footnotes (`[^1]`), definition lists, and math
//!   (`$x$` inline, `$$x$$` display).
//! - Front matter: YAML between `---` fences, or TOML between `+++` fences,
//!   at the very start of the file. It must be a mapping; it becomes the
//!   document's metadata.
//! - An explicit `{#id}` on a heading becomes its section's `id`, as does
//!   a `(label)=` target before it; a target elsewhere is an `anchor`. The
//!   host generates the other ids after parsing.
//! - Raw HTML becomes `raw` nodes with format `html`.
//! - Soft line breaks become `\n` within text; hard breaks become
//!   `line_break` nodes.
//! - Directives and roles, in MyST's syntax: a backtick, tilde or colon
//!   (`:::`) fence whose info string starts with `{name}` is a directive
//!   (the rest of the info line is its argument, leading `:option: value`
//!   lines or a `---` YAML block its options, the rest its content), and
//!   `` {name}`text` `` is a role. Those plugins declare (see "Syntax
//!   extensions" in `spec/wire.md`) are read by their declaration: options
//!   are checked and markup content is parsed in place, with the page's
//!   link references and footnotes. Other directives become custom nodes
//!   of their name with the content as text, as unknown reStructuredText
//!   directives do; other roles stay text and code.

mod convert;
mod lines;
mod md;
mod plugins;

use lsdocs_plugin_sdk::doctree::Document;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lsdocs_plugin_sdk::protocol::{Diagnostic, ParseParams, ParseResult, Severity, Syntax};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-myst", env!("CARGO_PKG_VERSION"))
        .parser(Myst)
        .cacheable()
}

/// The MyST parser.
#[derive(Debug, Clone, Copy, Default)]
pub struct Myst;

impl Parser for Myst {
    fn extensions(&self) -> Vec<String> {
        vec!["md".into(), "markdown".into()]
    }

    async fn parse(&self, params: ParseParams, _ctx: &Context) -> PluginResult<ParseResult> {
        let (document, diagnostics) =
            parse_with_syntax(&params.source, &params.text, &params.syntax);
        Ok(ParseResult {
            document,
            diagnostics,
            dependencies: Vec::new(),
        })
    }
}

/// Parses Markdown into a document, plus any problems found (e.g. invalid
/// front matter).
pub fn parse(source: &str, input: &str) -> (Document, Vec<Diagnostic>) {
    parse_with_syntax(source, input, &Syntax::default())
}

/// Parses Markdown as [`parse`] does, reading the directives and roles of
/// `syntax` by their declarations.
pub fn parse_with_syntax(
    source: &str,
    input: &str,
    syntax: &Syntax,
) -> (Document, Vec<Diagnostic>) {
    let text = lines::normalize(input);
    let md = md::parser(syntax);
    // markdown-it-rs 0.7 can overflow on some input (its emphasis rule):
    // in builds with overflow checks that panics; report it, don't crash.
    let parsed = catch_unwind(AssertUnwindSafe(|| md.parse(&text.text)));
    let (mut document, mut diagnostics) = match parsed {
        Ok(root) => convert::document(source, &text.text, &root),
        Err(_) => (
            Document::new(source),
            vec![Diagnostic {
                severity: Severity::Error,
                message: "the Markdown parser (markdown-it-rs) failed on this file".into(),
                source: Some(source.to_owned()),
                span: None,
                code: Some("parser-failed".into()),
            }],
        ),
    };
    text.map(&mut document, &mut diagnostics);
    (document, diagnostics)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use lsdocs_plugin_sdk::doctree::{Align, Node, Span, Text};
    use serde_json::json;

    /// The directives of `input`'s first top-level node, as JSON.
    fn mdit_json(input: &str) -> (Vec<Node>, serde_json::Value) {
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let value = serde_json::to_value(&doc.children).unwrap();
        (doc.children, value)
    }

    #[test]
    fn colon_fence_is_a_directive() {
        let (nodes, value) =
            mdit_json(":::{versionadded} 1.2 beta\n:class: x\n\n*New* here.\n:::\n");
        assert_eq!(value[0]["kind"], "versionadded");
        assert_eq!(
            value[0]["attrs"],
            json!({"argument": "1.2 beta", "options": {"class": "x"}})
        );
        assert_eq!(plain(nodes[0].children()), "New here.");
    }

    #[test]
    fn colon_fences_nest() {
        let (nodes, value) =
            mdit_json("::::{versionadded} 1\n:::{versionadded} 2\ninner\n:::\nouter\n::::\n");
        assert_eq!(nodes.len(), 1);
        let outer = nodes[0].children();
        assert_eq!(value[0]["children"][0]["attrs"]["argument"], "2");
        assert_eq!(plain(outer[0].children()), "inner");
        assert_eq!(plain(&outer[1..]), "outer");
    }

    #[test]
    fn unterminated_colon_fence_runs_to_end() {
        let (nodes, _) = mdit_json(":::{versionadded} 1\ntext\n\nmore\n");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].children().len(), 2);
        let (nodes, _) = mdit_json("> :::{versionadded} 1\n> x\n\nafter\n");
        assert_eq!(nodes.len(), 2);
        assert_eq!(plain(&nodes[1..]), "after");
    }

    #[test]
    fn directive_content_sees_page_references() {
        let (nodes, _) =
            mdit_json("```{versionadded} 1\n[x][r] and note[^n]\n```\n\n[r]: /u\n\n[^n]: N.\n");
        let inline = nodes[0].children()[0].children();
        let Node::Link(link) = &inline[0] else {
            panic!("{inline:?}")
        };
        assert_eq!(link.url, "/u");
        assert!(inline.iter().any(|n| n.kind() == "footnote_reference"));
    }

    /// `nodes` as `kind` or `kind(id)`, top level only.
    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| match n {
                Node::Anchor(a) => format!("anchor({})", a.id),
                Node::Section(s) => format!("section({})", s.id.as_deref().unwrap_or("-")),
                n => n.kind().into_owned(),
            })
            .collect()
    }

    #[test]
    fn target_names_next_section() {
        let (nodes, _) = mdit_json("(intro)=\n# Intro\n");
        assert_eq!(ids(&nodes), ["section(intro)"]);
        let (nodes, _) = mdit_json("(intro)=\n\n## Intro\n");
        assert_eq!(ids(&nodes), ["section(intro)"]);
    }

    #[test]
    fn target_elsewhere_is_anchor() {
        let input = "(a)=\n\ntext\n";
        let (nodes, _) = mdit_json(input);
        assert_eq!(ids(&nodes), ["anchor(a)", "paragraph"]);
        assert_eq!(at(input, &nodes[0]), "(a)=\n");
    }

    #[test]
    fn target_at_end_is_anchor() {
        let (nodes, _) = mdit_json("text\n(z)=\n");
        assert_eq!(ids(&nodes), ["paragraph", "anchor(z)"]);
    }

    #[test]
    fn explicit_heading_id_wins_over_target() {
        let (nodes, _) = mdit_json("(t)=\n# H {#x}\n");
        assert_eq!(ids(&nodes), ["anchor(t)", "section(x)"]);
    }

    #[test]
    fn footnote_and_math_edges() {
        // An undefined label is text; a definition's indented paragraph is
        // its own.
        let doc = parse_ok("Undefined[^none] here.\n");
        assert_eq!(plain_text_of(&doc.children[0]), "Undefined[^none] here.");
        let doc = parse_ok("x[^a]\n\n[^a]: First.\n\n    Second.\n");
        let Node::FootnoteDefinition(def) = &doc.children[1] else {
            panic!("{doc:?}")
        };
        assert_eq!(def.children.len(), 2);
        // `$` before a digit or after a space does not delimit math.
        let doc = parse_ok("Price $5 and $ x $ no.\n");
        assert_eq!(plain_text_of(&doc.children[0]), "Price $5 and $ x $ no.");
        let doc = parse_ok("$$\nx^2\n$$\n");
        let Node::Math(math) = &doc.children[0].children()[0] else {
            panic!("{doc:?}")
        };
        assert_eq!((math.value.as_str(), math.display), ("\nx^2\n", true));
    }

    #[test]
    fn alert_and_definition_edges() {
        let doc = parse_ok("> [!note]\n> lower\n");
        let Node::Admonition(alert) = &doc.children[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(alert.kind, "note");
        let doc = parse_ok("Term\n: One\n: Two\n\nOther\n: Def\n");
        let kinds: Vec<_> = doc.children[0]
            .children()
            .iter()
            .map(|n| n.kind())
            .collect();
        assert_eq!(
            kinds,
            [
                "definition_term",
                "definition_description",
                "definition_description",
                "definition_term",
                "definition_description"
            ]
        );
    }

    #[test]
    fn role_text_reads_as_code() {
        let (doc, _) = parse_with_syntax("t.md", "{kbd}` a  b `\n", &syntax());
        let value = serde_json::to_value(&doc.children[0].children()[0]).unwrap();
        assert_eq!(value["attrs"]["text"], "a  b");
    }

    /// Asserts every span in `doc` (and its diagnostics) lies in `input`
    /// on character boundaries.
    fn spans_in_input(input: &str, doc: &Document, diagnostics: &[Diagnostic]) {
        let mut spans: Vec<Span> = doc.descendants().filter_map(Node::span).collect();
        spans.extend(diagnostics.iter().filter_map(|d| d.span));
        for s in spans {
            let (a, b) = (s.start as usize, s.end as usize);
            assert!(
                a <= b
                    && b <= input.len()
                    && input.is_char_boundary(a)
                    && input.is_char_boundary(b),
                "span {a}..{b} outside {input:?}"
            );
        }
    }

    #[test]
    fn crlf_reads_as_lf_with_spans_in_the_file() {
        let input = "*a\r\nb*\r\n";
        let (doc, diagnostics) = parse("t.md", input);
        spans_in_input(input, &doc, &diagnostics);
        assert_eq!(at(input, &doc.children[0].children()[0]), "*a\r\nb*");
        let input = "# T\r\n\r\npara\r\n";
        let doc = parse_ok(input);
        let Node::Section(section) = &doc.children[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(at(input, &section.children[0]), "# T\r\n");
        assert_eq!(at(input, &section.children[1]), "para\r\n");
    }

    #[test]
    fn spans_stay_in_the_file() {
        for input in [
            "- *x*\n\ta*b*\n",
            "- *`*\n\ta",
            ">\t:",
            "- \n\t:\n",
            "Intro paragraph here.\n\nword \n*x*\n",
            "*\u{e9}\r\n]*",
        ] {
            let (doc, diagnostics) = parse("t.md", input);
            spans_in_input(input, &doc, &diagnostics);
        }
    }

    #[test]
    fn inline_argument_span_is_the_argument() {
        let input = "```{centered} e\n```\n";
        let (doc, _) = parse_with_syntax("t.md", input, &syntax());
        let argument = &doc.children[0].children()[0];
        assert_eq!(argument.span(), Some(Span::new(14, 15)));
    }

    #[test]
    fn lazy_lines_continue_footnotes_and_definitions() {
        let doc = parse_ok("x[^n]\n\n[^n]: a\nb\n");
        assert_eq!(doc.children.len(), 2, "{doc:?}");
        assert_eq!(plain(doc.children[1].children()), "a\nb");
        let doc = parse_ok("Term\n: def\nlazy\n");
        assert_eq!(doc.children.len(), 1, "{doc:?}");
        assert_eq!(plain(&doc.children[0].children()[1..]), "def\nlazy");
    }

    #[test]
    fn unclosed_math_and_footnotes_are_linear() {
        let started = std::time::Instant::now();
        parse_ok(&"$x [^a ".repeat(30_000));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    fn parse_ok(input: &str) -> Document {
        let (doc, diagnostics) = parse("t.md", input);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        doc
    }

    fn at(input: &str, node: &Node) -> String {
        let s = node.span().unwrap();
        input[s.start as usize..s.end as usize].to_owned()
    }

    #[test]
    fn block_structure_and_spans() {
        let input = "# Title\n\nSome *em* and **strong** with `code`.\n\n> quote\n\n---\n";
        let doc = parse_ok(input);
        let [Node::Section(section)] = &doc.children[..] else {
            panic!("{doc:?}")
        };
        let kinds: Vec<_> = section.children.iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["heading", "paragraph", "block_quote", "thematic_break"]
        );
        // The section spans its heading through its last child.
        assert_eq!(at(input, &doc.children[0]), input);
        assert_eq!(at(input, &section.children[0]), "# Title\n");
        let para = &section.children[1];
        assert_eq!(at(input, para), "Some *em* and **strong** with `code`.\n");
        let inline: Vec<_> = para.children().iter().map(|n| n.kind()).collect();
        assert_eq!(
            inline,
            [
                "text",
                "emphasis",
                "text",
                "strong",
                "text",
                "inline_code",
                "text"
            ]
        );
        assert_eq!(at(input, &para.children()[1]), "*em*");
    }

    #[test]
    fn yaml_and_toml_front_matter() {
        let doc = parse_ok("---\ntitle: Hello\ntags: [a, b]\nweight: 3\n---\n# Hi\n");
        assert_eq!(doc.metadata["title"], json!("Hello"));
        assert_eq!(doc.metadata["tags"], json!(["a", "b"]));
        assert_eq!(doc.metadata["weight"], json!(3));
        assert_eq!(doc.children.len(), 1);

        let doc = parse_ok("+++\ntitle = \"Plus\"\n+++\ntext\n");
        assert_eq!(doc.metadata["title"], json!("Plus"));
    }

    #[test]
    fn bad_front_matter_is_a_warning() {
        let (doc, diagnostics) = parse("t.md", "---\ntitle: [unclosed\n---\n# Hi\n");
        assert_eq!(doc.children.len(), 1);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(
            diagnostics[0]
                .message
                .starts_with("invalid YAML front matter"),
            "{diagnostics:?}"
        );
        assert_eq!(diagnostics[0].span.unwrap().start, 0);

        let (_, diagnostics) = parse("t.md", "---\n- just\n- a list\n---\n");
        assert!(
            diagnostics[0].message.contains("must be a mapping"),
            "{diagnostics:?}"
        );
    }

    /// Sections as `title(id)[subsections]`, with `-` for no id.
    fn outline(nodes: &[Node]) -> String {
        nodes
            .iter()
            .filter_map(|n| match n {
                Node::Section(s) => {
                    let Some(Node::Heading(h)) = s.children.first() else {
                        panic!("a section starts with its heading: {s:?}")
                    };
                    assert_eq!(h.id, None, "the id belongs to the section");
                    let inner = outline(&s.children[1..]);
                    let inner = if inner.is_empty() {
                        inner
                    } else {
                        format!("[{inner}]")
                    };
                    Some(format!(
                        "{}({}){inner}",
                        plain(&h.children),
                        s.id.as_deref().unwrap_or("-")
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn plain(nodes: &[Node]) -> String {
        lsdocs_plugin_sdk::doctree::plain_text(nodes)
    }

    #[test]
    fn headings_open_nested_sections() {
        let input = "intro\n\n# A\n\na\n\n## B {#bee}\n\n#### D\n\n### C\n\n# E\n\n> ## Quoted\n";
        let doc = parse_ok(input);
        assert_eq!(doc.children[0].kind(), "paragraph");
        assert_eq!(outline(&doc.children), "A(-)[B(bee)[D(-) C(-)]] E(-)");
        // A heading in a block quote is not a section.
        let Node::Section(e) = &doc.children[2] else {
            panic!()
        };
        assert_eq!(e.children[1].children()[0].kind(), "heading");
        assert_eq!(
            at(input, &doc.children[1]),
            "# A\n\na\n\n## B {#bee}\n\n#### D\n\n### C\n"
        );
    }

    #[test]
    fn alerts_footnotes_definitions_and_math() {
        let input = "> [!WARNING]\n> Careful.\n\nSee[^n] $a+b$ and\n$$x^2$$\n\nTerm\n: Meaning\n\n[^n]: The note.\n";
        let doc = parse_ok(input);
        let kinds: Vec<_> = doc.descendants().map(|n| n.kind().into_owned()).collect();
        for kind in [
            "admonition",
            "footnote_reference",
            "math",
            "definition_list",
            "definition_term",
            "definition_description",
            "footnote_definition",
        ] {
            assert!(kinds.iter().any(|k| k == kind), "{kind} missing: {kinds:?}");
        }
        let Node::Admonition(alert) = &doc.children[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(alert.kind, "warning");
        assert_eq!(plain(&alert.children), "Careful.");
        let maths: Vec<_> = doc
            .descendants()
            .filter_map(|n| match n {
                Node::Math(m) => Some((m.value.as_str(), m.display)),
                _ => None,
            })
            .collect();
        assert_eq!(maths, [("a+b", false), ("x^2", true)]);
        let label = doc.descendants().find_map(|n| match n {
            Node::FootnoteDefinition(f) => Some(f.label.as_str()),
            _ => None,
        });
        assert_eq!(label, Some("n"));
    }

    fn syntax() -> Syntax {
        serde_json::from_value(json!({
            "directives": [
                {"name": "versionadded", "required_arguments": 1,
                 "final_argument_whitespace": true,
                 "options": {"class": "value", "hidden": "flag"}, "content": "blocks"},
                {"name": "centered", "required_arguments": 1,
                 "final_argument_whitespace": true, "argument": "inline"},
                {"name": "code-block", "optional_arguments": 1, "content": "text"},
            ],
            "roles": [{"name": "kbd"}],
        }))
        .unwrap()
    }

    #[test]
    fn declared_directives_and_roles_become_custom_nodes() {
        let input = "```{versionadded} 1.2 beta\n:class: x\n:hidden:\n\n*New* here.\n```\n\n\
                     ```{centered} Hello *world*\n```\n\n\
                     ```{code-block} rust\nfn main() {}\n```\n\n\
                     Press {kbd}`Ctrl+C` or `code`.\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let value = serde_json::to_value(&doc.children).unwrap();
        assert_eq!(value[0]["kind"], "versionadded");
        assert_eq!(
            value[0]["attrs"],
            json!({"argument": "1.2 beta", "options": {"class": "x", "hidden": true}})
        );
        // Content is parsed, with spans in the file.
        let Node::Custom(added) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(at(input, &added.children[0]), "*New* here.");
        assert_eq!(at(input, &added.children[0].children()[0]), "*New*");
        assert_eq!(value[1]["kind"], "centered");
        let Node::Custom(centered) = &doc.children[1] else {
            panic!()
        };
        let argument = &centered.children[0];
        assert_eq!(value[1]["children"][0]["kind"], "directive_argument");
        assert_eq!(at(input, argument), "Hello *world*");
        assert_eq!(at(input, &argument.children()[1]), "*world*");
        assert_eq!(
            value[2]["attrs"],
            json!({"argument": "rust", "content": "fn main() {}"})
        );
        let para = &doc.children[3];
        let kinds: Vec<_> = para.children().iter().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            ["text", "custom:role", "text", "inline_code", "text"]
        );
        assert_eq!(at(input, &para.children()[1]), "{kbd}`Ctrl+C`");
        assert_eq!(
            value[3]["children"][1]["attrs"],
            json!({"name": "kbd", "text": "Ctrl+C"})
        );
        assert_eq!(plain_text_of(para), "Press Ctrl+C or code.");
    }

    fn plain_text_of(node: &Node) -> String {
        lsdocs_plugin_sdk::doctree::plain_text(node.children())
    }

    #[test]
    fn directive_options_in_yaml_and_problems() {
        let input = "```{versionadded} 2\n---\nclass: big\nhidden: true\n---\nText.\n```\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let value = serde_json::to_value(&doc.children[0]).unwrap();
        assert_eq!(
            value["attrs"]["options"],
            json!({"class": "big", "hidden": true})
        );
        assert_eq!(value["children"][0]["type"], "paragraph");
        let input = "```{versionadded}\n:nope: 1\n:hidden: yes\n```\n";
        let (_, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        let messages: Vec<_> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "directive \"versionadded\": 1 argument(s) required, 0 supplied",
                "directive \"versionadded\": unknown option: \"nope\"",
                "directive \"versionadded\": option \"hidden\" takes no value",
            ]
        );
        assert!(diagnostics.iter().all(|d| d.span.is_some()));
    }

    #[test]
    fn undeclared_directives_and_roles() {
        let input = "```{mermaid} arg\n:theme: dark\n\ngraph TD; A-->B\n```\n\n{kbd}`x`\n";
        let doc = parse_ok(input);
        let value = serde_json::to_value(&doc.children).unwrap();
        assert_eq!(value[0]["kind"], "mermaid");
        assert_eq!(
            value[0]["attrs"],
            json!({"argument": "arg", "options": {"theme": "dark"}, "content": "graph TD; A-->B"})
        );
        let kinds: Vec<_> = doc.children[1]
            .children()
            .iter()
            .map(|n| n.kind())
            .collect();
        assert_eq!(kinds, ["text", "inline_code"]);
    }

    #[test]
    fn directives_nest_in_containers() {
        let input = "> ```{versionadded} 1\n> Inside **quote**.\n> ```\n";
        let (doc, diagnostics) = parse_with_syntax("t.md", input, &syntax());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let quote = &doc.children[0];
        let Node::Custom(added) = &quote.children()[0] else {
            panic!("{doc:?}")
        };
        let strong = &added.children[0].children()[1];
        assert_eq!(at(input, strong), "**quote**");
    }

    #[test]
    fn code_blocks() {
        let doc = parse_ok("```rust title=\"x.rs\"\nfn main() {}\n```\n\n    indented\n");
        let Node::CodeBlock(fenced) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(fenced.language.as_deref(), Some("rust"));
        assert_eq!(fenced.info.as_deref(), Some("title=\"x.rs\""));
        assert_eq!(fenced.value, "fn main() {}\n");
        let Node::CodeBlock(indented) = &doc.children[1] else {
            panic!()
        };
        assert_eq!(
            (indented.language.as_deref(), indented.value.as_str()),
            (None, "indented\n")
        );
    }

    #[test]
    fn lists_and_task_items() {
        let doc = parse_ok("3. three\n4. four\n\n- [x] done\n- [ ] todo\n");
        let Node::List(ordered) = &doc.children[0] else {
            panic!()
        };
        assert!(ordered.ordered);
        assert_eq!(ordered.start, Some(3));
        let Node::List(tasks) = &doc.children[1] else {
            panic!()
        };
        let checked: Vec<_> = tasks
            .children
            .iter()
            .map(|n| match n {
                Node::ListItem(i) => i.checked,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(checked, [Some(true), Some(false)]);
    }

    #[test]
    fn tables() {
        let doc = parse_ok("| a | b |\n|:--|--:|\n| 1 | 2 |\n");
        let Node::Table(table) = &doc.children[0] else {
            panic!()
        };
        assert_eq!(table.align, [Align::Left, Align::Right]);
        let rows: Vec<bool> = table
            .children
            .iter()
            .map(|r| match r {
                Node::TableRow(r) => r.header,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(rows, [true, false]);
        let Node::Text(a) = &table.children[0].children()[0].children()[0] else {
            panic!()
        };
        assert_eq!(a.value, "a");
    }

    #[test]
    fn links_images_html_and_breaks() {
        let doc = parse_ok(
            "[docs](guide.md \"Guide\") ![alt *text*](a.png)  \nnext <b>x</b>\n\n<div>\nblock\n</div>\n",
        );
        let para = doc.children[0].children();
        let Node::Link(link) = &para[0] else {
            panic!("{para:?}")
        };
        assert_eq!(
            (link.url.as_str(), link.title.as_deref()),
            ("guide.md", Some("Guide"))
        );
        let Node::Image(image) = &para[2] else {
            panic!("{para:?}")
        };
        assert_eq!(
            (image.url.as_str(), image.alt.as_str()),
            ("a.png", "alt text")
        );
        assert!(para.iter().any(|n| matches!(n, Node::LineBreak(_))));
        assert!(
            para.iter()
                .any(|n| matches!(n, Node::Raw(r) if r.value == "<b>"))
        );
        let Node::Raw(block) = &doc.children[1] else {
            panic!()
        };
        assert_eq!(block.value, "<div>\nblock\n</div>\n");
    }

    #[test]
    fn soft_breaks_merge_into_text() {
        let doc = parse_ok("one\ntwo\n");
        assert_eq!(
            doc.children[0].children(),
            [Node::Text(Text {
                span: Some(Span::new(0, 7)),
                value: "one\ntwo".into()
            })]
        );
    }
}
