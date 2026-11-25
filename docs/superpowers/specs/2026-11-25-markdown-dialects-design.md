# Markdown dialects as features - design

## Intent

lsdocs-markdown connects Markdown parsers to lsdocs. Markdown has several
dialects: `md-vanilla` (the pulldown-cmark library's Markdown: CommonMark
plus its extensions), MyST, and later pymdownx-style syntax. Each dialect becomes its own lsdocs
parser plugin, selected at compile time by a Cargo feature and at site
level by which plugin the site lists.

## Goals

- `lsdocs-parser-markdown` has one Cargo feature per dialect, all on by
  default; building with none of them is a compile error.
- Each dialect is its own lsdocs plugin with its own `plugin()`:
  - `md-vanilla` feature -> plugin `parser-md-vanilla`, implemented in
    this crate on pulldown-cmark.
  - `myst` feature -> plugin `parser-myst`, implemented in a new
    workspace crate `crates/parser-myst` (`lsdocs-parser-myst`), which
    `lsdocs-parser-markdown` depends on optionally and re-exports.
- The MyST dialect keeps today's behavior and tests unchanged.

## Non-goals

- Moving MyST to a markdown-it fork (later; `parser-myst` may become a
  wrapper around that fork).
- The pymdownx dialect (later; possibly wrapping zensical's parser).

## Constraints

- AGENTS.md conventions: workspace dependencies, no panics outside tests,
  ASCII docs, LF, `missing_docs`, `nur ci` passes, `nur deny` passes
  with all features.
- MSRV may be bumped if needed (not expected for this change).

## Approaches considered

1. One plugin with a runtime `dialect` setting (no host change; all
   dialects always compiled together). Not chosen.
2. **One plugin per dialect (chosen by the user).** Explicit at site
   level; each dialect has its own name, version line and cache key.
3. A crate per dialect with features in the host. Not chosen: dialect
   features belong here, the crate that interfaces with lsdocs.

## Design

- `crates/parser-myst`: today's `parser-markdown/src/lib.rs` moved over
  (git mv to keep history), plugin name `parser-myst`, same public
  `parse`/`parse_with_syntax`, same tests. CHANGELOG of its own.
- `crates/parser-markdown`:
  - `[features] default = ["md-vanilla", "myst"]`,
    `md-vanilla = ["dep:pulldown-cmark"]`,
    `myst = ["dep:lsdocs-parser-myst"]`.
  - `lib.rs`: crate docs, `compile_error!` when no dialect feature is
    on, `#[cfg(feature = "md-vanilla")] pub mod vanilla;`,
    `#[cfg(feature = "myst")] pub use lsdocs_parser_myst as myst;`.
  - `vanilla.rs`: `plugin()` (`parser-md-vanilla`), `parse()`, and a
    trimmed copy of the pulldown-cmark -> document builder without
    directives and roles. It enables pulldown-cmark's extensions the
    builder maps today (tables, task lists, GFM alerts, footnotes,
    definition lists, math, heading attributes, YAML/TOML front
    matter) plus strikethrough.
  - Settings (`.config/lsdocs/<name>/settings.toml`, read with the SDK's
    `settings_toml_keys`): `smart-punctuation` (bool), off by default as
    docutils' `smart_quotes` is.
  - Tests for vanilla: block structure and spans, GFM features,
    `{#id}` headings, front matter, `{name}` fences staying code
    blocks and `{name}` roles staying text and code, smart punctuation
    off by default and on when set.
- `scripts/org.nuon`: add `lsdocs-parser-myst` under `lsdocs-markdown`.
- README and AGENTS.md: list the dialects and features; Known
  limitations and TODO updated (MyST migration to a markdown-it fork,
  pymdownx).
- Other repositories, each on a `feat/markdown-dialects` branch with a
  PR: the host (`lsdocs/lsdocs`) registers `parser-md-vanilla` and
  `parser-myst` in place of `parser-markdown`; site configs naming
  `parser-markdown` (e.g. lsdocs-plugin-sdk's `api/lsdocs.toml`) move
  to `parser-myst`, which keeps today's output.

The builder duplication between the two crates is temporary: it ends
when MyST moves to markdown-it.

## Left to the builder

- Exact split of shared helpers inside `vanilla.rs`.