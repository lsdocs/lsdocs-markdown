# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The MyST parser, moved from `lsdocs-parser-markdown` and registered as
  `parser-myst`.
- Parsing with markdown-it (`markdown-it-rs`) in place of pulldown-cmark,
  with MyST's syntax as markdown-it rules; output as before.
- MyST colon fences (`:::{name}`), `(label)=` targets, and declared
  directive content parsed in place, with the page's link references and
  footnotes.
