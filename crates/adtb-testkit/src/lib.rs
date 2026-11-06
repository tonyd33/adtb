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
