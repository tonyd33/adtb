use std::collections::BTreeSet;

use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

use super::{Status, Transactions, Writers, Xid};

/// What a pruner decides of one version from its writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// Dead to every snapshot.
    Remove,
    /// Its deleter aborted: clear its deleter, `next` and successor flags, then judge it again.
    Clear,
    /// Visible to some snapshot but not to every one.
    Keep,
    /// Visible to every snapshot, live or taken later, until the version is next written.
    AllVisible,
}

/// Which versions a horizon removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Of a current heap: those dead to every snapshot, except those the retention window keeps.
    Current,
    /// Of a current heap: those dead to every snapshot, the ones the retention window keeps
    /// included, which the caller first moves to history.
    Moving,
    /// Of a history heap: those the retention window no longer keeps.
    Past,
}

/// What the versions of one heap are judged by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Horizon {
    watermark: Xid,
    /// The least commit time the retention window keeps, if history is kept.
    cutoff: Option<u64>,
    rule: Rule,
}

impl<R: Runtime> Transactions<R> {
    /// Returns the horizon on-access pruning of heap `heap` judges by now.
    pub fn horizon_now(&self, heap: u32) -> Horizon {
        Horizon {
            watermark: self.watermark(heap),
            cutoff: self.cutoff(),
            rule: Rule::Current,
        }
    }

    /// Returns the horizon a vacuum judges heap `heap` by for the whole of its run: the least of
    /// the heap's watermark and the oldest transaction running, with `cutoff` and `rule`.
    ///
    /// Postcondition: a version it finds dead stays dead however the heap's watermark and the
    /// window move.
    pub fn horizon_fixed(&self, heap: u32, cutoff: Option<u64>, rule: Rule) -> Horizon {
        let w = self.watermark(heap);
        Horizon {
            watermark: self.running().first().map_or(w, |&r| w.min(r)),
            cutoff,
            rule,
        }
    }

    /// Returns the fate of a version written as `w` in a heap judged by `horizon`.
    ///
    /// Under [`Rule::Past`] it reads no status: a history version is removed once the window no
    /// longer keeps its deleter.
    pub fn fate(
        &self,
        pager: &Pager<R>,
        w: &Writers,
        horizon: &Horizon,
    ) -> Result<Fate, PageError> {
        let inserter = w.inserter();
        let deleter = w.deleter();
        let keeps = |x: Xid| self.retains(pager, x, horizon.cutoff);
        Ok(match horizon.rule {
            Rule::Past if keeps(deleter.unwrap_or(Xid::NONE))? => Fate::Keep,
            Rule::Past => Fate::Remove,
            Rule::Current | Rule::Moving => match self.status(pager, inserter)? {
                Status::Aborted => Fate::Remove,
                Status::InProgress => Fate::Keep,
                Status::Committed => match deleter {
                    None if inserter < horizon.watermark => Fate::AllVisible,
                    None => Fate::Keep,
                    Some(x) => match self.status(pager, x)? {
                        Status::Aborted => Fate::Clear,
                        Status::Committed if x >= horizon.watermark => Fate::Keep,
                        Status::Committed
                            if horizon.rule == Rule::Current
                                && self.window_keeps(pager, w, x, horizon, || Ok(true))? =>
                        {
                            Fate::Keep
                        }
                        Status::Committed => Fate::Remove,
                        Status::InProgress => Fate::Keep,
                    },
                },
            },
        })
    }

    /// Returns whether the retention window bounded by `horizon` keeps a version written as `w`
    /// once it is dead to every snapshot: its inserter committed, and another transaction deleted
    /// it and committed below the watermark within the window.
    pub fn kept_by_window(
        &self,
        pager: &Pager<R>,
        w: &Writers,
        horizon: &Horizon,
    ) -> Result<bool, PageError> {
        let Some(x) = w.deleter() else {
            return Ok(false);
        };
        if x >= horizon.watermark {
            return Ok(false);
        }
        self.window_keeps(pager, w, x, horizon, || {
            Ok(self.status(pager, w.inserter())? == Status::Committed
                && self.status(pager, x)? == Status::Committed)
        })
    }

    /// Returns whether the window bounded by `horizon` keeps a version written as `w` and deleted
    /// by `x`: `x` is not its inserter, `committed` confirms both writers committed, and `x`
    /// committed at or after the cutoff. `committed` runs only once `x` is known not to be the
    /// inserter.
    fn window_keeps(
        &self,
        pager: &Pager<R>,
        w: &Writers,
        x: Xid,
        horizon: &Horizon,
        committed: impl FnOnce() -> Result<bool, PageError>,
    ) -> Result<bool, PageError> {
        Ok(!w.inserted_by(x) && committed()? && self.retains(pager, x, horizon.cutoff)?)
    }

    /// Returns the [`Transactions::fate`] under `horizon` of each version's writers it is given.
    pub fn judge<'a>(
        &'a self,
        pager: &'a Pager<R>,
        horizon: Horizon,
    ) -> impl FnMut(&Writers) -> Result<Fate, PageError> + 'a {
        move |w| self.fate(pager, w, &horizon)
    }

    /// Returns whether `x` has committed or aborted.
    pub fn resolved(&self, pager: &Pager<R>, x: Xid) -> Result<bool, PageError> {
        Ok(self.status(pager, x)? != Status::InProgress)
    }

    /// Returns whether the inserter of a version written as `w` committed and its deleter, if any,
    /// did not, each settled by its status or, when its status page is damaged, by `logged`, or
    /// `None` when a writer is settled by neither.
    pub fn latest_live(
        &self,
        pager: &Pager<R>,
        w: &Writers,
        logged: &BTreeSet<u64>,
    ) -> Option<bool> {
        let gone = match w.deleter() {
            Some(x) => self.settled(pager, logged, x)?,
            None => false,
        };
        Some(self.settled(pager, logged, w.inserter())? && !gone)
    }
}
