# lsdocs-markdown

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

The lsdocs plugins parsing Markdown into lsdocs' shared document shape,
one per dialect. `crates/parser-markdown` (`lsdocs-parser-markdown`)
has a Cargo feature per dialect, all on by default (at least one must be
enabled); a site lists the plugin of the dialect it is written in.

| Feature | Plugin | Dialect |
|---|---|---|
| `pulldown-cmark` | `parser-pulldown-cmark` | Markdown as the `pulldown-cmark` library reads it: CommonMark with GitHub tables, task lists and alerts, footnotes, definition lists, math, explicit heading ids and YAML/TOML front matter. Setting: `smart-punctuation` (off by default). |
| `myst` | `parser-myst` | MyST, on markdown-it (the `markdown-it-rs` crate, the parser MyST is built on): the same constructs, plus directives in ```` ```{name} ```` and `:::{name}` fences (declared ones' content parsed in place, with the page's link references and footnotes), `` {name}`text` `` roles and `(label)=` targets. From `crates/parser-myst` (`lsdocs-parser-myst`). |

Settings go in `.config/lsdocs/<name>/settings.toml`, `name` being the
plugin's name in the site config:

```toml
smart-punctuation = true
```

## Known limitations

- Strikethrough (`~~x~~`) is not read: the shared document shape has no
  node for it.
- `parser-myst` has no `smart-punctuation` setting.
- `parser-myst` reads only part of MyST's syntax: no substitutions,
  `{attrs}`, `$$` equation labels, `%` comments or `+++` block breaks.
- `parser-myst` works around bugs in `markdown-it-rs` 0.7 (fixed on its
  unreleased main branch): `\r\n` line endings are read as `\n` (spans
  still point into the file); spans its source maps get wrong (tabs in
  container indentation, some trailing spaces) are left out; and on the
  input where its emphasis rule overflows (e.g. tab-indented list
  continuations), builds with overflow checks report a `parser-failed`
  error for the file instead of crashing.
- `parser-myst` keeps pulldown-cmark's quirks where its output did
  (spans, footnote and math syntax), so sites read as before the move to
  markdown-it.

## TODO

- [ ] A pymdownx dialect (perhaps on zensical's parser).
- [ ] More MyST syntax as markdown-it rules (see Known limitations).
- [ ] GFM's remaining extensions in `parser-myst` (markdown-it-rs has
      strikethrough and linkify rules): strikethrough needs a node in the
      shared document shape; then bare URL autolinks.
- [ ] `markdown-it-rs`'s next release (breaking changes): its footnote and
      math plugins, once their node types are public, could replace ours.
