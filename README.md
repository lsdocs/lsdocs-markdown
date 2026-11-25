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
| `myst` | `parser-myst` | The same, plus syntax extensions (directives and roles) in MyST's syntax: ```` ```{name} ```` fences and `` {name}`text` `` roles. From `crates/parser-myst` (`lsdocs-parser-myst`). |

Settings go in `.config/lsdocs/<name>/settings.toml`, `name` being the
plugin's name in the site config:

```toml
smart-punctuation = true
```

## Known limitations

- Strikethrough (`~~x~~`) is not read: the shared document shape has no
  node for it.
- `parser-myst` has no `smart-punctuation` setting.
- MyST's colon fences (`:::{name}`) are not read (pulldown-cmark has no
  hook for them), nor MyST's `(label)=` targets.
- Directive content is parsed on its own, so it cannot use the page's
  link reference definitions or footnotes.

## TODO

- [ ] `parser-myst` on markdown-it (a maintained fork of the `markdown-it`
      crate: 0.6.1 fails cargo-deny on RUSTSEC-2024-0421 through `mdurl`),
      which also brings the two items below.
- [ ] A pymdownx dialect (perhaps on zensical's parser).
- [ ] MyST colon fences and `(label)=` targets.
- [ ] Directive content parsed with the page's link reference definitions
      and footnotes.
