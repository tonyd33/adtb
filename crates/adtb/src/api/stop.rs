//! Cancellation tokens, deadlines and statement limits: what stops a call before it ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::runtime::Runtime;
use crate::write::Bounds;

use super::error::{Error, RejectReason};

/// A token that stops every call carrying it. Clones share one token; once cancelled it stays
/// cancelled.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl PartialEq for CancelToken {
    fn eq(&self, other: &CancelToken) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CancelToken {}

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// Stop every call carrying the token, from any thread. Each returns `Cancelled`, and a call
    /// started with it later returns `Cancelled` without running.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// What stops one call: its token, its deadline and its statement limit.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Stops<'a> {
    pub(super) cancel: Option<&'a CancelToken>,
    pub(super) deadline: Option<SystemTime>,
    pub(super) limit: Option<Duration>,
}

impl<'a> Stops<'a> {
    /// Refuses a call whose token is cancelled with `Cancelled`, and one whose deadline has
    /// passed on `rt`'s wall clock with `DeadlineExceeded`.
    pub(super) fn check<R: Runtime>(&self, rt: &R) -> Result<(), Error> {
        if self.cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(Error::rejected(RejectReason::Cancelled));
        }
        if self.deadline.is_some_and(|d| d <= rt.wall()) {
            return Err(Error::rejected(RejectReason::DeadlineExceeded));
        }
        Ok(())
    }

    /// Returns the bounds of the call's statements on `rt`, each fixpoint bounded by
    /// `fix_rounds`.
    pub(super) fn bounds<R: Runtime>(&self, rt: &R, fix_rounds: Option<usize>) -> Bounds {
        let deadline = self.deadline.map(|d| {
            let left = d.duration_since(rt.wall()).unwrap_or(Duration::ZERO);
            rt.now() + left
        });
        Bounds {
            fix_rounds,
            limit: self.limit,
            cancel: self.cancel.map(|t| t.0.clone()),
            deadline,
        }
    }

    /// Returns the earlier deadline and the shorter limit of `self` and `other`, and the token
    /// of `self`, else of `other`.
    pub(super) fn within(self, other: Stops<'a>) -> Stops<'a> {
        Stops {
            cancel: self.cancel.or(other.cancel),
            deadline: least(self.deadline, other.deadline),
            limit: least(self.limit, other.limit),
        }
    }
}

fn least<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
