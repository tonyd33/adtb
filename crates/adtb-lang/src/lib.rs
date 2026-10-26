#![forbid(unsafe_code)]
#![allow(clippy::large_enum_variant)]

pub mod analysis;
pub mod ast;
pub mod check;
pub mod collation;
pub mod evolve;
pub mod ids;
pub mod kernel;
pub mod lex;
pub mod norm;
pub mod parse;
pub mod resolve;
pub mod schema;
pub mod span;
pub mod types;
pub mod visit;
