use crate::runtime::Runtime;
use crate::storage::{PageError, PageExclusive, Pager, Pid, Shape};

use super::Refused;
use super::dir::{names_at, names_page, vm_root};
use super::stage::Stage;
use super::write::Rank;

/// The page to refuse for a page out of shape, and what shows the fault still holds under its
/// exclusive latch.
pub(super) struct Blame {
    rank: Rank,
    shape: Shape,
    holds: Holds,
}

pub(super) enum Holds {
    /// The blamed page names this page, the one out of shape.
    Names(Pid),
    /// The blamed page is these bytes, judged out of shape.
    Is(Vec<u8>),
    /// The blamed page, a directory's root, names no visibility map.
    NoMap,
    /// The blamed level-0 directory page names `page` at entry `at`, and level-0 directory page
    /// `first` names it too, at entry `first_at`.
    Twice {
        page: Pid,
        at: usize,
        first: Pid,
        first_at: usize,
    },
}

impl Blame {
    pub(super) fn new(rank: Rank, shape: Shape, holds: Holds) -> Blame {
        Blame { rank, shape, holds }
    }

    pub(super) fn pid(&self) -> Pid {
        self.rank.pid()
    }

    /// Returns the blame of the page `rank` names, read out of `shape` as `node`: of `parent`, the
    /// page naming it, when there is one, and of the page itself otherwise.
    pub(super) fn of(rank: Rank, parent: Option<Rank>, shape: Shape, node: Vec<u8>) -> Blame {
        match parent {
            Some(parent) => Blame {
                rank: parent,
                shape,
                holds: Holds::Names(rank.pid()),
            },
            None => Blame {
                rank,
                shape,
                holds: Holds::Is(node),
            },
        }
    }

    /// Refuse the blamed page as out of shape: under the latch `stage` holds on it when its staged
    /// copy is unchanged, or else under its exclusive latch when the fault still holds there,
    /// first releasing the latches `stage` holds at or above its rank. When one of those latches
    /// holds a changed copy, abandon the stage's change and release every latch it holds first.
    /// Returns `None` when the fault no longer holds and the stage is whole: the caller reads
    /// again; [`Refused::Gone`] when it no longer holds and the stage was abandoned: the caller's
    /// change fails as changed.
    pub(super) fn refuse<'p, R: Runtime>(
        self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Option<Refused> {
        let pid = self.rank.pid();
        if stage.holds(pid) && !stage.changed(pid) {
            return Some(Refused::Page(stage.refuse(pid, self.shape)));
        }
        let whole = stage.release_from(self.rank);
        if !whole {
            stage.abandon();
        }
        let judge = |p: &PageExclusive<'_, R>| {
            let holds = match &self.holds {
                Holds::Names(pid) => names_page(p, *pid),
                Holds::Is(bytes) => p[..] == bytes[..],
                Holds::NoMap => vm_root(p).is_none(),
                Holds::Twice {
                    page,
                    at,
                    first,
                    first_at,
                } => {
                    names_at(p, *at, *page)
                        && if *first == pid {
                            names_at(p, *first_at, *page)
                        } else {
                            pager.try_optimistic(*first).is_some_and(|seen| {
                                seen.read(|node| names_at(node, *first_at, *page))
                                    && seen.validate()
                            })
                        }
                }
            };
            holds.then_some(self.shape)
        };
        match pager.refuse_if(pid, judge) {
            Some(e) => Some(Refused::Page(e)),
            None if whole => None,
            None => Some(Refused::Gone),
        }
    }

    /// Refuse the blamed page as [`Blame::refuse`] does, through a stage that holds no changed copy
    /// ranked at or above the blamed page. Returns `None` when the fault no longer holds.
    pub(super) fn refuse_unchanged<'p, R: Runtime>(
        self,
        stage: &mut Stage<'p, R>,
        pager: &'p Pager<R>,
    ) -> Option<PageError> {
        self.refuse(stage, pager).map(unchanged)
    }
}

/// Returns the page error of `refused`, a failure through a stage that held no changed copy
/// ranked at or above the blamed page.
pub(super) fn unchanged(refused: Refused) -> PageError {
    match refused {
        Refused::Page(e) => e,
        Refused::Gone => unreachable!(
            "a stage with no changed copy ranked at or above the blamed page is never abandoned"
        ),
    }
}
