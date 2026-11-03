//! The fixtures, generators and sweep sizes the test crates share.

pub mod cases;
pub mod corpus;
pub mod draw;
pub mod fixture;
pub mod fuzz;
pub mod inhabit;
pub mod profile;
pub mod running;
pub mod scratch;
pub mod sections;
pub mod spec;
pub mod writes;

mod rng;

pub use rng::Rng;

/// Assert that `got` is an error printing exactly `reason`.
#[track_caller]
pub fn refused_with<T, E: std::fmt::Display>(got: Result<T, E>, reason: &str) {
    match got {
        Ok(_) => panic!("accepted; expected the refusal `{reason}`"),
        Err(e) => assert_eq!(reason, e.to_string()),
    }
}

/// Returns the expression of the key of identity `n` of the running example.
pub fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}
