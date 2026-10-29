pub mod cases;
pub mod corpus;
pub mod draw;
pub mod fixture;
pub mod fuzz;
pub mod inhabit;
pub mod profile;
pub mod running;
pub mod spec;
pub mod writes;

mod rng;

pub use rng::Rng;

/// Returns the expression of the key of identity `n` of the running example.
pub fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}
