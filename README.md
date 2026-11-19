# lsdocs-markdown

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

`crates/parser-markdown` (`lsdocs-parser-markdown`): the lsdocs plugin
parsing Markdown. CommonMark via `pulldown-cmark` into lsdocs' shared
document shape, with GitHub tables, task lists and alerts, footnotes,
definition lists, math, explicit heading ids, and syntax extensions
(directives and roles) in MyST's syntax: ```` ```{name} ```` fences and
`` {name}`text` `` roles.

## Known limitations

- MyST's colon fences (`:::{name}`) are not read (pulldown-cmark has no
  hook for them), nor MyST's `(label)=` targets.
- Directive content is parsed on its own, so it cannot use the page's
  link reference definitions or footnotes.

## TODO

- [ ] MyST colon fences and `(label)=` targets.
- [ ] Directive content parsed with the page's link reference definitions
      and footnotes.
