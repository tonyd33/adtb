use crate::kernel::{Kind, Terms, Tm};

use super::{Cause, Error, Root};

/// The most loops a normal form nests: a generator, an aggregate, a grouping, a top or a fixpoint.
const MAX_LOOPS: u32 = 64;

/// The most nested normalising steps of a root, within any thread's default stack.
const MAX_RECURSION: u32 = 128;

/// The most terms on a path down a normal form, within any thread's default stack.
const MAX_HEIGHT: u32 = 128;

/// The terms and steps a root may spend whatever its size. Each also gets a multiple of the terms
/// it has as input.
const BASE_SPEND: usize = 1 << 18;
const SPEND_PER_TERM: usize = 4;

/// The most terms on a path down a term the engine plans and runs, within a worker's stack.
const MAX_RUN_HEIGHT: u32 = 64;

#[cfg(feature = "testing")]
static RUN_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(MAX_RUN_HEIGHT);

/// Set the most terms on a path down a term the engine plans and runs, for this process, to
/// `height`: a build with other limits.
#[cfg(feature = "testing")]
pub fn set_run_height(height: u32) {
    RUN_HEIGHT.store(height, std::sync::atomic::Ordering::Relaxed);
}

fn run_height() -> u32 {
    #[cfg(feature = "testing")]
    return RUN_HEIGHT.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "testing"))]
    MAX_RUN_HEIGHT
}

/// Fail on the first of `roots` that, written out with no term shared, passes what the engine
/// plans and runs.
pub(super) fn within_run_bound(
    terms: &Terms,
    roots: impl IntoIterator<Item = (Root, Tm)>,
) -> Result<(), Error> {
    let bound = RunBound::of(terms);
    match roots.into_iter().find(|(_, t)| !bound.holds(*t)) {
        Some((root, _)) => Err(Error {
            root,
            cause: Cause::TooLarge,
        }),
        None => Ok(()),
    }
}

/// The written out size and the height of each term of an arena.
struct RunBound {
    limit: u64,
    shapes: Vec<(u64, u32)>,
}

impl RunBound {
    fn of(terms: &Terms) -> RunBound {
        let mut shapes: Vec<(u64, u32)> = Vec::with_capacity(terms.len());
        for at in 0..terms.len() {
            let kind = terms.kind(Tm(at as u32));
            let size = kind
                .children()
                .iter()
                .fold(1u64, |n, c| n.saturating_add(shapes[c.0 as usize].0));
            shapes.push((size, stack_height(kind, |c| shapes[c.0 as usize].1)));
        }
        RunBound {
            limit: (BASE_SPEND + SPEND_PER_TERM * terms.len()) as u64,
            shapes,
        }
    }

    fn holds(&self, root: Tm) -> bool {
        let (size, height) = self.shapes[root.0 as usize];
        size <= self.limit && height <= run_height()
    }
}

/// Returns the height of a term given each child's `height`. A block's body stands at the block's
/// height, so every walk of a block must take its statements in a loop.
fn stack_height(kind: &Kind, height: impl Fn(Tm) -> u32) -> u32 {
    match kind {
        Kind::Block(write, body) => (height(*write) + 1).max(height(*body)),
        _ => kind
            .children()
            .iter()
            .map(|c| height(*c) + 1)
            .max()
            .unwrap_or(0),
    }
}

/// What normalising one root has spent.
#[derive(Default)]
pub(super) struct Budget {
    /// The loops nested in each output term and its height, up to the terms measured so far.
    shapes: Vec<(u32, u32)>,
    /// The output arena's length when the root began.
    base: usize,
    steps: usize,
    pub(super) depth: u32,
    limit: usize,
}

impl Budget {
    pub(super) fn new(input_terms: usize) -> Budget {
        Budget {
            limit: BASE_SPEND + SPEND_PER_TERM * input_terms,
            ..Budget::default()
        }
    }

    pub(super) fn start(&mut self, out: &Terms) {
        self.base = out.len();
        self.steps = 0;
    }

    /// Refuse a root past any limit.
    pub(super) fn spend(&mut self, out: &Terms) -> Result<(), Cause> {
        self.steps += 1;
        for at in self.shapes.len()..out.len() {
            let at = Tm(at as u32);
            let inner = out
                .children(at)
                .iter()
                .map(|c| self.shapes[c.0 as usize].0)
                .max()
                .unwrap_or(0);
            let height = stack_height(out.kind(at), |c| self.shapes[c.0 as usize].1);
            let own = matches!(
                out.kind(at),
                Kind::For(..)
                    | Kind::Agg { .. }
                    | Kind::Group { .. }
                    | Kind::Top { .. }
                    | Kind::Fix(..)
            );
            let nested = inner + u32::from(own);
            if nested > MAX_LOOPS || height > MAX_HEIGHT {
                return Err(Cause::TooLarge);
            }
            self.shapes.push((nested, height));
        }
        if self.depth > MAX_RECURSION
            || self.steps > self.limit
            || out.len() - self.base > self.limit
        {
            return Err(Cause::TooLarge);
        }
        Ok(())
    }
}
