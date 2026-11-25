//! First-party Markdown parser plugins, one per dialect, each behind a
//! Cargo feature of the same name (all on by default; at least one must
//! be enabled). Each is its own lsdocs plugin, with its own `plugin()`:
//!
//! - `pulldown-cmark`: [`pulldown_cmark`], the `parser-pulldown-cmark`
//!   plugin (Markdown as the `pulldown-cmark` library reads it).
//! - `myst`: [`myst`], the `parser-myst` plugin (MyST's directives and
//!   roles), from `lsdocs-parser-myst`.

#[cfg(not(any(feature = "pulldown-cmark", feature = "myst")))]
compile_error!(
    "lsdocs-parser-markdown: enable at least one dialect feature (`pulldown-cmark`, `myst`)"
);

#[cfg(feature = "pulldown-cmark")]
pub mod pulldown_cmark;

#[cfg(feature = "myst")]
pub use lsdocs_parser_myst as myst;
