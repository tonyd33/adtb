//! The engine-free helpers the tests share: fixtures, generators, sweep sizes, byte damage, the
//! storage design's mark table and the isolation checker. The `sim` feature adds the crash loops,
//! failure searches, the loading of rows through a session and runs of a simulation, which need
//! `adtb-sim` and `adtb-sys`.
#![forbid(unsafe_code)]

#[cfg(feature = "sim")]
pub mod backends;
pub mod cases;
#[cfg(feature = "sim")]
pub mod common;
pub mod corpus;
#[cfg(feature = "sim")]
pub mod crash;
pub mod damage;
pub mod draw;
#[cfg(feature = "sim")]
pub mod entropy;
pub mod fixture;
pub mod fuzz;
pub mod inhabit;
pub mod isolation;
#[cfg(feature = "sim")]
pub mod load;
pub mod marks;
#[cfg(feature = "sim")]
pub mod process;
pub mod profile;
pub mod running;
pub mod schemas;
pub mod scratch;
#[cfg(feature = "sim")]
pub mod search;
pub mod sections;
pub mod seeds;
pub mod spec;
pub mod writes;

mod rng;

pub use rng::Rng;

/// Assert that `got` is the refusal `expected` and prints exactly `reason`.
#[track_caller]
pub fn refused_as<T, E>(got: Result<T, E>, expected: &E, reason: &str)
where
    T: std::fmt::Debug,
    E: PartialEq + std::fmt::Debug + std::fmt::Display,
{
    match got {
        Ok(value) => panic!("accepted {value:?}; expected the refusal {expected:?}: `{reason}`"),
        Err(e) => {
            assert_eq!(expected, &e);
            assert_eq!(reason, e.to_string());
        }
    }
}

/// Returns the expression of the key of identity `n` of the running example.
pub fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}

#[cfg(test)]
mod tests {
    use super::refused_as;
    use std::fmt;
    use std::panic::{UnwindSafe, catch_unwind};

    #[derive(Debug, PartialEq)]
    enum Refusal {
        Full,
        Late,
    }

    impl fmt::Display for Refusal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{self:?} refusal")
        }
    }

    fn panic_of(check: impl FnOnce() + UnwindSafe) -> Option<String> {
        catch_unwind(check)
            .err()
            .map(|p| p.downcast_ref::<String>().cloned().unwrap_or_default())
    }

    #[test]
    fn only_the_refusal_with_its_whole_reason_passes() {
        let full = || Err::<u8, _>(Refusal::Full);
        assert_eq!(
            None,
            panic_of(|| refused_as(full(), &Refusal::Full, "Full refusal"))
        );
        assert_eq!(
            Some("accepted 1; expected the refusal Full: `Full refusal`".to_owned()),
            panic_of(|| refused_as(Ok(1), &Refusal::Full, "Full refusal"))
        );
        assert!(panic_of(|| refused_as(full(), &Refusal::Late, "Full refusal")).is_some());
        assert!(panic_of(|| refused_as(full(), &Refusal::Full, "Full")).is_some());
    }
}
