use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::Location;

use super::Step;
use crate::storage::{Damage, PageError, Pid};

/// Where a step runs: the source place of the call that runs it, and the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    file: &'static str,
    line: u32,
    step: Step,
}

impl Site {
    pub fn step(self) -> Step {
        self.step
    }

    /// Returns the name of the file the site is in, without its directory.
    pub fn file_name(self) -> &'static str {
        self.file.rsplit(['/', '\\']).next().unwrap_or(self.file)
    }
}

/// One run of a step: its site, the how-many-th run of that site on this thread since the
/// last [`reached`], and whether it was failed.
pub type Run = (Site, usize, bool);

#[derive(Default)]
struct State {
    fail: Option<(Site, usize)>,
    runs: BTreeMap<Site, usize>,
    reached: Vec<Run>,
    watching: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Fail run `occurrence` of `site` on this thread, counting from 0 at the last [`reached`].
pub fn fail(site: Site, occurrence: usize) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.watching = true;
        s.fail = Some((site, occurrence));
    });
}

/// Returns every run of a step on this thread since the last call, and starts the count
/// again. No run is counted or failed on a thread before its first call or [`fail`].
pub fn reached() -> Vec<Run> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.watching = true;
        s.runs.clear();
        std::mem::take(&mut s.reached)
    })
}

#[track_caller]
pub fn struck(step: Step) -> bool {
    let at = Location::caller();
    let site = Site {
        file: at.file(),
        line: at.line(),
        step,
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.watching {
            return false;
        }
        let n = *s.runs.entry(site).and_modify(|n| *n += 1).or_insert(0);
        let struck = s.fail == Some((site, n));
        if struck {
            s.fail = None;
        }
        s.reached.push((site, n, struck));
        struck
    })
}

/// Returns a read the device refused when a test failed this run of `step` at its caller's place.
#[track_caller]
pub fn injected(step: Step) -> Option<PageError> {
    struck(step).then_some(PageError {
        pid: Pid(0),
        damage: Damage::Unreadable,
    })
}
