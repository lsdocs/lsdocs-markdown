//! First-party Markdown parser plugins, one per dialect, each behind a
//! Cargo feature of the same name (all on by default; at least one must
//! be enabled):
//!
//! - `myst`: [`myst`], the `parser-myst` plugin (MyST's directives and
//!   roles), from `lsdocs-parser-myst`.

#[cfg(not(feature = "myst"))]
compile_error!("lsdocs-parser-markdown: enable at least one dialect feature (`myst`)");

#[cfg(feature = "myst")]
pub use lsdocs_parser_myst as myst;
