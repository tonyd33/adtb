#![forbid(unsafe_code)]
#![expect(
    clippy::large_enum_variant,
    reason = "the syntax tree and schema enums keep their large variants unboxed"
)]

pub mod analysis;
pub mod ast;
pub mod calendar;
pub mod canon;
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
pub mod textual;
pub mod types;
pub mod visit;
