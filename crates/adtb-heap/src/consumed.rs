//! A guard that a value is consumed before it drops.

/// A guard whose drop panics in a debug build unless [`Consumed::consume`] ran first.
#[derive(Debug, PartialEq, Eq)]
pub struct Consumed {
    armed: bool,
}

impl Consumed {
    pub fn armed() -> Consumed {
        Consumed { armed: true }
    }

    pub fn consume(mut self) {
        self.armed = false;
    }
}

impl Drop for Consumed {
    fn drop(&mut self) {
        if cfg!(debug_assertions) && self.armed && !std::thread::panicking() {
            panic!("a value that must be consumed dropped unconsumed");
        }
    }
}
