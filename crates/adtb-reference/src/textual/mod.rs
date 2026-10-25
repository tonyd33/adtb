//! The textual syntax of values: one canonical printed form per value, read back at a known
//! type, and the versioned documents that hold rows of one relation variable.

mod document;
mod print;
mod read;

pub use document::{Document, Header, print_document, read_document};
pub(crate) use print::to_text;
pub use print::{print, self_key};
pub use read::{ParseError, parse};
