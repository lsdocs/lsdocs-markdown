# parser-myst on markdown-it - design

## Intent

`lsdocs-parser-myst` (the `myst` dialect, plugin `parser-myst`) moves from
pulldown-cmark to markdown-it, the parser MyST itself is built on
(markdown-it-py + mdit-py-plugins). markdown-it's block and inline rules
are pluggable, so MyST syntax becomes plugins instead of patches applied
after parsing. That fixes today's known limitations (colon fences,
`(label)=` targets, directive content parsed on its own) and makes
further MyST syntax a matter of adding rules.

## Decisions (the user's)

- The parser is upstream `markdown-it-rs` from crates.io (GSGFs7's fork
  of markdown-it-rust/markdown-it: the actively maintained one; MIT;
  passes cargo-deny). No lsdocs fork for now; fixes upstream are a later
  question.
- Footnotes and math are our own plugins: 0.7.0's footnote and math node
  types are private, so a converter cannot read them.
- `md-vanilla` stays on pulldown-cmark.

## Goals

- `parser-myst` on `markdown-it-rs` 0.7 (`default-features = false`; no
  linkify, no syntect), no pulldown-cmark.
- Every current `parser-myst` test passes unchanged (output, spans and
  diagnostics), except where a test pins a limitation being removed.
- New: MyST colon fences (`:::{name}` ... `:::`, same directive rules as
  backtick fences), `(label)=` targets, and directive content parsed in
  place, so it uses the page's link reference definitions and footnotes.
- `nur ci`, `nur msrv` (Rust 1.89; 0.7.0 builds on it) and `nur deny` pass.

## Non-goals

- Other MyST syntax (substitutions, `{attrs}`, `$$` equation labels,
  `%` comments, block breaks `+++`); later, as plugins.
- Strikethrough (no node in the document shape), linkify, typographer.
- Following upstream's unreleased breaking changes (pinned to 0.7).

## Design

`crates/parser-myst/src/`:

- `lib.rs`: `plugin()`, `Myst`, `parse`, `parse_with_syntax` (signatures
  unchanged); builds the `MarkdownIt` once per call with the declared
  `Syntax`, parses, converts.
- `md.rs`: the `MarkdownIt` setup: `cmark`, `html`, `tables`, `tasklist`,
  `front_matter` (YAML/TOML; parsed with serde-saphyr / toml as today),
  plus our plugins; source maps on.
- `plugins/`: one file per rule set, each defining public node types the
  converter downcasts:
  - `footnote.rs`: `[^label]` references and `[^label]: ...` definitions
    (definitions stay where written, as today).
  - `math.rs`: `$x$` inline and `$$x$$` display, as pulldown-cmark reads
    them today.
  - `deflist.rs`: definition lists (pulldown-cmark's syntax: term line,
    then `: definition`).
  - `alert.rs`: `> [!NOTE]` etc. block quotes become admonitions.
  - `heading_id.rs`: a trailing `{#id}` on a heading.
  - `directive.rs`: backtick fences whose info is `{name}` and colon
    fences `:::{name}`; argument, `:key: value` / `---` YAML options and
    content split as today; declared `Blocks` content tokenized in place
    by the block parser (shared references and footnotes); declared
    inline arguments parsed by the inline parser.
  - `role.rs`: `` {name}`text` `` for declared roles.
  - `target.rs`: `(label)=` lines.
- `convert.rs`: markdown-it AST -> lsdocs nodes with spans from source
  maps, then sectionize (moved from today's code). A target directly
  before a heading becomes that section's id; elsewhere it is an
  `anchor` node with that id.

Diagnostics (front matter, directive options/arguments/content) keep
their codes and messages.

## Left to the builder

- How spans are normalized where markdown-it's source maps differ from
  pulldown-cmark's (the existing tests fix the expected spans).
- Internal plugin structure and rule ordering.
