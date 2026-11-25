//! The markdown-it rules MyST needs beyond CommonMark, each with node
//! types of its own for the converter to read.

pub(crate) mod deflist;
pub(crate) mod directive;
pub(crate) mod footnote;
pub(crate) mod math;
pub(crate) mod role;
pub(crate) mod target;
