//! First-party Markdown parser plugins, one per dialect, each behind a
//! Cargo feature of the same name (all on by default; at least one must
//! be enabled). Each is its own lsdocs plugin, with its own `plugin()`:
//!
//! - `md-vanilla`: [`vanilla`], the `parser-md-vanilla` plugin (the
//!   `pulldown-cmark` library's Markdown).
//! - `myst`: [`myst`], the `parser-myst` plugin (MyST's directives and
//!   roles), from `lsdocs-parser-myst`.

#[cfg(not(any(feature = "md-vanilla", feature = "myst")))]
compile_error!(
    "lsdocs-parser-markdown: enable at least one dialect feature (`md-vanilla`, `myst`)"
);

#[cfg(feature = "md-vanilla")]
pub mod vanilla;

#[cfg(feature = "myst")]
pub use lsdocs_parser_myst as myst;
