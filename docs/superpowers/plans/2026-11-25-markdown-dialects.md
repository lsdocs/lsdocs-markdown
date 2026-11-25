# Markdown dialects Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the Markdown parser into per-dialect lsdocs plugins
(`parser-md-vanilla`, `parser-myst`) behind Cargo features, and move the
host and site configs to the new names.

**Architecture:** `crates/parser-myst` takes today's parser unchanged
(plugin renamed). `crates/parser-markdown` becomes the umbrella: feature
`md-vanilla` (its own pulldown-cmark module) and feature `myst`
(re-export of `lsdocs-parser-myst`). The host registers one builtin per
dialect.

**Tech Stack:** Rust 2024, pulldown-cmark 0.13, lsdocs-plugin-sdk, nur.

**Spec:** `docs/superpowers/specs/2026-11-25-markdown-dialects-design.md`

## Global Constraints

- Features: `default = ["md-vanilla", "myst"]`; no dialect feature ->
  `compile_error!`.
- Plugin names: `parser-md-vanilla`, `parser-myst`.
- `md-vanilla` setting `smart-punctuation` (bool), default `false`, in
  `.config/lsdocs/<name>/settings.toml`.
- AGENTS.md conventions (workspace deps, no panics outside tests, ASCII
  docs, LF, Conventional Commits); `nur ci` and `nur deny` pass.
- Branch `feat/markdown-dialects` in each repository; one PR each.

## Review Focus

- Vanilla parser given MyST syntax: ```` ```{note} ```` is a plain
  `code_block` with language `{note}`; `` {r}`x` `` is text + code. Test
  in Task 2.
- Unknown/ill-typed settings key (`smart-punctuation = "yes"`) fails
  `initialize` with the file named (SDK `settings_toml_keys` does this;
  Task 2 test via `Plugin::setup`).
- Building `--no-default-features --features myst` and `--features
  md-vanilla` alone compiles (Task 3 verification).
- `parser-myst` output byte-identical to before (its tests move
  unchanged; Task 1).
- Host sites naming `parser-markdown` get the "no builtin named" error
  listing the new names (Task 4 test update).

---

### Task 1: Move the MyST parser into `crates/parser-myst`

**Files:**
- Create: `crates/parser-myst/Cargo.toml`, `crates/parser-myst/CHANGELOG.md`
- Move: `crates/parser-markdown/src/lib.rs` -> `crates/parser-myst/src/lib.rs` (`git mv`)
- Modify: `Cargo.toml` (workspace dep `lsdocs-parser-myst = { path = "crates/parser-myst", version = "0.1.0" }`), `scripts/org.nuon`

**Interfaces:**
- Produces: `lsdocs_parser_myst::{plugin() -> Plugin, Myst, parse(&str, &str) -> (Document, Vec<Diagnostic>), parse_with_syntax(&str, &str, &Syntax) -> (Document, Vec<Diagnostic>)}`; plugin name `parser-myst`.

- [ ] `git mv`, rename `Markdown` -> `Myst`, plugin name -> `parser-myst`, crate docs say "MyST".
- [ ] Run `cargo test -p lsdocs-parser-myst`: 14 tests pass.
- [ ] Commit `refactor(parser-myst): move the MyST parser into its own crate`.

### Task 2: `md-vanilla` module

**Files:**
- Create: `crates/parser-markdown/src/vanilla.rs`
- Test: in `vanilla.rs` (`mod tests`)

**Interfaces:**
- Produces: `vanilla::{plugin() -> Plugin, Vanilla, Settings { smart_punctuation: bool }, parse(source: &str, input: &str, settings: &Settings) -> (Document, Vec<Diagnostic>)}`.

- [ ] Tests (fail first): `block_structure_and_spans` (as MyST's),
  `gfm_extensions` (table, task list, `~~s~~` -> `strikethrough` kind as
  the builder maps it, alert -> `admonition`, footnote, deflist, math),
  `heading_id`, `front_matter`, `myst_syntax_is_plain_markdown`
  (`{note}` fence is `code_block` with language `{note}`; role is text +
  `inline_code`), `smart_punctuation_off_by_default` (`"a"` stays `"a"`),
  `smart_punctuation_when_set` (`"a"` -> `\u{201c}a\u{201d}`),
  `bad_setting_fails_setup`.
- [ ] Implement: copy of the builder minus directive/role code; options =
  MyST's set + `ENABLE_STRIKETHROUGH` (+ `ENABLE_SMART_PUNCTUATION` when
  set); loader `settings_toml_keys(c, &[("smart-punctuation", Kind::Bool)])`;
  `cacheable()`.
- [ ] `cargo test -p lsdocs-parser-markdown` passes; commit
  `feat(parser-markdown): md-vanilla dialect`.

### Task 3: Features, docs, CI

**Files:** `crates/parser-markdown/{Cargo.toml,src/lib.rs,CHANGELOG.md}`, `README.md`, `AGENTS.md`, `nurfile` if CI should build each feature alone.

- [ ] Features per Global Constraints; `lib.rs`: `compile_error!`,
  `pub mod vanilla`, `pub use lsdocs_parser_myst as myst`.
- [ ] Verify: `cargo check -p lsdocs-parser-markdown --no-default-features`
  fails with the compile_error; `--features md-vanilla` and
  `--features myst` alone build; `nur ci` and `nur deny` pass.
- [ ] Commit `feat(parser-markdown): dialects as features`; push; PR.

### Task 4: Host (`lsdocs/lsdocs`)

**Files:** `crates/cli/{Cargo.toml,src/main.rs,tests/*.rs}`,
`tests/site/lsdocs.toml`, `tests/site/docs/guide/configuration.md`,
snapshots, `scripts/org.nuon`, `README.md`.

- [ ] Features `parser-md-vanilla = ["dep:lsdocs-parser-markdown", "lsdocs-parser-markdown/md-vanilla"]`, `parser-myst = [..."/myst"]`; dep `default-features = false`; builtins `parser-md-vanilla` -> `vanilla::plugin`, `parser-myst` -> `myst::plugin`; starter config and test sites -> `parser-myst`.
- [ ] Verify with `nur link ../lsdocs-markdown`: `nur ci` passes (snapshots updated only for the name). PR notes it needs the markdown PR merged then `nur bump lsdocs-parser-markdown`.

### Task 5: SDK (`lsdocs/lsdocs-plugin-sdk`)

- [ ] `api/lsdocs.toml`, `tests/site/guide/configuration.md`, binding toc-port tests, `scripts/org.nuon` -> `parser-myst` / add `lsdocs-parser-myst`. PR notes it needs the host PR merged.
