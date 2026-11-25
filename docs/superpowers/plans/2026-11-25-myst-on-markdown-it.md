# parser-myst on markdown-it Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `lsdocs-parser-myst` on `markdown-it-rs` 0.7 with MyST
syntax as markdown-it plugins, same output as today plus colon fences,
`(label)=` targets and directive content parsed in place.

**Architecture:** A new markdown-it pipeline (`md.rs` + `plugins/` +
`convert.rs`) is built next to today's pulldown-cmark builder. While both
exist, a differential test helper parses the same input with both and
asserts equal documents and diagnostics. The last task switches `parse`
to the new pipeline and deletes the pulldown-cmark code.

**Tech Stack:** Rust 2024, `markdown-it-rs` 0.7.0 (`markdown_it` lib,
`default-features = false`), lsdocs-plugin-sdk.

**Spec:** `docs/superpowers/specs/2026-11-25-myst-on-markdown-it-design.md`

## Global Constraints

- `markdown-it-rs = { version = "0.7.0", default-features = false }` in
  `[workspace.dependencies]`; no linkify, no syntect.
- MSRV stays the workspace's 1.89; `nur ci`, `nur msrv`, `nur deny` pass.
- Public API unchanged: `plugin()`, `Myst`, `parse(&str, &str)`,
  `parse_with_syntax(&str, &str, &Syntax)`; plugin name `parser-myst`.
- Spans: byte offsets into the source, end exclusive, equal to today's.
  markdown-it block source maps stop before the line ending; pulldown's
  include it (spike: paragraph `(9, 46)` vs today's `"...`code`.\n"`).
- Diagnostic codes/messages unchanged (`front-matter`, `directive`).
- AGENTS.md conventions: no panics outside tests, ASCII docs, LF.

## Review Focus

- Unterminated colon fence (`:::{note}` with no closing `:::`): content
  runs to the end of its container, as an unterminated backtick fence
  does. Test in Task 4.
- A role or directive name not declared: role stays text + inline code;
  directive becomes a custom node with content as text (today's
  behavior; covered by the differential tests).
- Directive content using a reference defined elsewhere on the page
  (`[x][ref]` inside, `[ref]: /u` outside) resolves. Test in Task 4.
- `(label)=` not followed by anything (end of file) is an `anchor`.
  Test in Task 5.
- Nested colon fences with more colons (`::::{a}` containing `:::{b}`)
  close correctly. Test in Task 4.

---

### Task 1: Pipeline scaffold, core nodes, differential tests

**Files:**
- Modify: `Cargo.toml` (workspace dep), `crates/parser-myst/Cargo.toml`
- Create: `crates/parser-myst/src/md.rs`, `src/convert.rs`
- Modify: `crates/parser-myst/src/lib.rs` (modules, test helper)

**Interfaces:**
- Produces: `md::parser(syntax: &Syntax) -> markdown_it::MarkdownIt`
  (cmark, html, tables, tasklist, front_matter; plugins added by later
  tasks register here); `convert::document(source: &str, input: &str,
  root: &markdown_it::Node) -> (Document, Vec<Diagnostic>)`;
  `fn parse_mdit(source, input, syntax) -> (Document, Vec<Diagnostic>)`
  (crate-private until Task 6). `sectionize` moves to `convert.rs`
  shared by both pipelines.
- Test helper in `lib.rs` tests: `fn same(input: &str)` and
  `fn same_with(input: &str, syntax: &Syntax)` asserting
  `parse_with_syntax(...) == parse_mdit(...)` (documents and
  diagnostics).

- [ ] Write tests `same_core` calling `same` on each input of today's
  `block_structure_and_spans`, `yaml_and_toml_front_matter`,
  `bad_front_matter_is_a_warning`, `headings_open_nested_sections`,
  `code_blocks`, `lists_and_task_items`, `tables`,
  `links_images_html_and_breaks`, `soft_breaks_merge_into_text`.
- [ ] Run `cargo test -p lsdocs-parser-myst same_core`: fails
  (`parse_mdit` missing).
- [ ] Implement: converter maps cmark/html/tables/tasklist nodes to the
  doctree kinds the pulldown builder emits; block spans extend over the
  trailing line ending when the source has one there; adjacent text
  merges as today; front matter via `FrontMatter::parse_with` with
  serde-saphyr/toml and today's warnings.
- [ ] `same_core` passes; all 14 existing tests still pass.
- [ ] Commit `refactor(parser-myst): markdown-it pipeline for CommonMark`.

### Task 2: Footnote and math plugins

**Files:** Create `src/plugins/mod.rs`, `src/plugins/footnote.rs`,
`src/plugins/math.rs`; modify `md.rs`, `convert.rs`.

**Interfaces:**
- Produces: `pub struct FootnoteReference { pub label: String }`,
  `pub struct FootnoteDefinition { pub label: String }` (children are
  its blocks), `pub struct Math { pub display: bool, pub value: String }`;
  `footnote::add(&mut MarkdownIt)`, `math::add(&mut MarkdownIt)`.

- [ ] Test `same_footnotes_and_math`: `same` on the footnote and math
  parts of `alerts_footnotes_definitions_and_math`, plus a footnote
  definition with an indented second paragraph and `$$` spanning lines.
- [ ] Fails; implement (pulldown-cmark's syntax: `[^l]`, `[^l]: ...`
  with 4-space continuation; `$x$` not followed by a digit-adjacent
  `$`, `$$x$$` inline or block); passes; commit
  `feat(parser-myst): footnote and math plugins`.

### Task 3: Definition lists, alerts, heading ids

**Files:** Create `src/plugins/{deflist,alert,heading_id}.rs`; modify
`md.rs`, `convert.rs`.

**Interfaces:**
- Produces: `DefinitionList`, `DefinitionTerm`, `DefinitionDescription`
  node types; `Alert { pub kind: &'static str }` (`note`, `tip`,
  `important`, `warning`, `caution`); heading id stored as
  `HeadingId(String)` in the heading node's `ext`; `add` fns per file.

- [ ] Test `same_definitions_alerts_ids`: `same` on the rest of
  `alerts_footnotes_definitions_and_math`, `# T {#top}`, and an alert
  with a lowercase `[!note]`.
- [ ] Fails; implement; passes; commit
  `feat(parser-myst): definition list, alert and heading id plugins`.

### Task 4: Directives (backtick and colon fences) and roles

**Files:** Create `src/plugins/{directive,role}.rs`; modify `md.rs`,
`convert.rs`; move `directive_options`, `check_arguments`, `is_name`
into `directive.rs` (shared by both pipelines until Task 6).

**Interfaces:**
- Consumes: `Syntax` stored in `md.ext` by `md::parser`.
- Produces: `Directive { name, argument, options, content: Option<String>,
  problems: Vec<String> }` node (children: parsed content, plus a leading
  `DirectiveArgument` node for inline arguments); `Role { name, text }`.

- [ ] Tests: `same_with` on the inputs of
  `declared_directives_and_roles_become_custom_nodes`,
  `directive_options_in_yaml_and_problems`,
  `undeclared_directives_and_roles`, `directives_nest_in_containers`.
  New (mdit only): `colon_fence_is_a_directive` (`:::{note}\nbody\n:::`
  equals the backtick form's document, spans aside),
  `colon_fences_nest` (`::::{note}\n:::{tip}\nx\n:::\n::::`),
  `unterminated_colon_fence_runs_to_end`,
  `directive_content_sees_page_references`
  (`` ```{note}\n[x][r]\n```\n\n[r]: /u `` gives a link to `/u`).
- [ ] Fails; implement: declared `Blocks` content tokenized in place
  (`state.md.block.tokenize` over the content lines, as blockquote does);
  inline arguments parsed with the inline parser; same diagnostics.
- [ ] Passes; commit `feat(parser-myst): directive and role plugins,
  colon fences`.

### Task 5: `(label)=` targets

**Files:** Create `src/plugins/target.rs`; modify `md.rs`, `convert.rs`.

**Interfaces:** Produces `Target { pub label: String }` block node.

- [ ] Tests (mdit only): `target_names_next_section`
  (`(intro)=\n# Intro\n` -> section id `intro`, no anchor node);
  `target_elsewhere_is_anchor` (`(a)=\n\ntext\n` -> `anchor` id `a`
  then paragraph); `target_at_end_is_anchor`; a target before a heading
  with `{#x}`: `{#x}` wins and the target is an anchor.
- [ ] Fails; implement; passes; commit
  `feat(parser-myst): (label)= targets`.

### Task 6: Switch over; drop pulldown-cmark

**Files:** `crates/parser-myst/{Cargo.toml,src/lib.rs,CHANGELOG.md}`,
`README.md`, `AGENTS.md`.

- [ ] `parse_with_syntax` uses the markdown-it pipeline; delete the
  pulldown builder and `same`/`same_with` (differential tests become
  plain tests asserting today's expected values, i.e. the 14 originals
  plus the new ones); drop `pulldown-cmark` from parser-myst.
- [ ] README: parser-myst row says markdown-it; remove the two fixed
  Known limitations and TODO items (colon fences/targets, directive
  content scope); TODO: other MyST syntax. CHANGELOG entry.
- [ ] `nur ci`, `nur msrv`, `cargo deny check` pass; commit
  `feat(parser-myst): parse with markdown-it`; push; PR into
  `feat/markdown-dialects`.
