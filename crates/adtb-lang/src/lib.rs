#![forbid(unsafe_code)]
#![allow(clippy::large_enum_variant)]

pub mod ast;
pub mod check;
pub mod kernel;
pub mod lex;
pub mod parse;
pub mod resolve;
pub mod schema;
pub mod span;
pub mod types;
pub mod visit;
