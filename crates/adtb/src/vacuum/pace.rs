use std::time::Duration;

use adtb_sys::lock::Lock;

use crate::runtime::{Instant, Runtime};

/// The longest one step may make a worker sleep.
const LONGEST: Duration = Duration::from_secs(30 * 24 * 3600);

/// The share of time the engine's maintenance may spend working, charged by every worker.
#[derive(Debug)]
pub struct Pacer {
    state: Lock<Paced>,
}

#[derive(Debug)]
struct Paced {
    budget: f64,
    /// The earliest instant the next step may start.
    next: Option<Instant>,
    charged: Charged,
}

/// The steps charged since the budget last changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charged {
    pub steps: u64,
    /// The time they took together.
    pub busy: Duration,
    pub longest: Duration,
    /// When the first of them began.
    pub since: Option<Instant>,
}

impl Paced {
    fn new(budget: f64) -> Paced {
        Paced {
            budget,
            next: None,
            charged: Charged::default(),
        }
    }
}

impl Pacer {
    pub fn new(budget: f64) -> Pacer {
        Pacer {
            state: Lock::new(Paced::new(budget)),
        }
    }

    /// Take `budget`, a fraction of time from 0 to 1, for every later step. A new budget
    /// forgets what earlier steps took.
    pub fn set<R: Runtime>(&self, rt: &R, budget: f64) {
        let mut st = self.state.lock(rt);
        if st.budget.to_bits() != budget.to_bits() {
            *st = Paced::new(budget);
        }
    }

    #[cfg(feature = "testing")]
    pub(crate) fn charged<R: Runtime>(&self, rt: &R) -> Charged {
        self.state.lock(rt).charged
    }

    /// Charge a step that began at `began` and ends now. Returns how long the worker sleeps
    /// before its next step, so that every worker's steps together take at most the budget of
    /// the time since the first of them, or `None` when the budget allows no work.
    pub fn charge<R: Runtime>(&self, rt: &R, began: Instant) -> Option<Duration> {
        let now = rt.now();
        let mut st = self.state.lock(rt);
        let spent = now.saturating_since(began);
        let c = &mut st.charged;
        c.steps += 1;
        c.busy += spent;
        c.longest = c.longest.max(spent);
        c.since = Some(c.since.map_or(began, |s| s.min(began)));
        let budget = Some(st.budget.min(1.0)).filter(|b| *b > 0.0)?;
        let from = st.next.map_or(began, |n| n.max(began));
        let next = from
            + Duration::try_from_secs_f64(spent.as_secs_f64() / budget)
                .map_or(LONGEST, |d| d.min(LONGEST));
        st.next = Some(next);
        Some(next.saturating_since(now))
    }
}
