use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;
use std::time::Duration;

use adtb_sim::{Outcome, Sim, SimRuntime};

/// The start of the message a simulated process panics with when a write or force fails.
pub const FATAL: &str = "fatal I/O error";

/// Keep panics that start with [`FATAL`] off stderr for the rest of the process.
pub fn silence_fatal_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("");
            if !msg.starts_with(FATAL) {
                default(info);
            }
        }));
    });
}

/// Run `body` as a process, treating a crash caused by a failed write or force as a crash.
pub fn run_process<F: FnOnce(SimRuntime)>(sim: &mut Sim, body: F) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| sim.run(body))) {
        Ok(o) => o,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            if msg.starts_with(FATAL) {
                Outcome::Crashed
            } else {
                panic::resume_unwind(payload)
            }
        }
    }
}

/// Returns how long round `round` of seed `seed` runs before its crash: `base` microseconds and
/// a spread below `span` more.
pub fn crash_after(seed: u64, round: u64, base: u64, span: u64) -> Duration {
    Duration::from_micros(base + (seed * 7919 + round * 104_729) % span)
}
