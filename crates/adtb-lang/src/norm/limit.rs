use crate::kernel::{Kind, Terms, Tm};

use super::{Error, NormalProgram, Root};

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

/// Returns whether each of `roots`, written out with no term shared, stays within what the engine
/// plans and runs.
pub fn expands_within_bound(terms: &Terms, roots: &[Tm]) -> bool {
    let bound = RunBound::of(terms);
    roots.iter().all(|r| bound.holds(*r))
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
        size <= self.limit && height <= MAX_RUN_HEIGHT
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

const PAST_RUN_BOUND: &str = "the text expands past what the engine plans";

pub(super) const TOO_LARGE: &str = "the text expands past what the engine normalises";

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
    pub(super) fn spend(&mut self, out: &Terms) -> Result<(), String> {
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
                return Err(TOO_LARGE.to_owned());
            }
            self.shapes.push((nested, height));
        }
        if self.depth > MAX_RECURSION
            || self.steps > self.limit
            || out.len() - self.base > self.limit
        {
            return Err(TOO_LARGE.to_owned());
        }
        Ok(())
    }
}

/// Fail on the first body of `p` that the engine plans and that passes the run bound. A function
/// is planned only inlined into its caller, which passes the bound itself.
pub(super) fn planned_within_bound(p: &NormalProgram) -> Result<(), Error> {
    let bound = RunBound::of(&p.terms);
    let holds = |root: Root, t: Tm| {
        if bound.holds(t) {
            Ok(())
        } else {
            Err(Error {
                root,
                message: PAST_RUN_BOUND.to_owned(),
            })
        }
    };
    for (c, t) in p.constraints() {
        holds(Root::Constraint(c), t)?;
    }
    for (c, t) in p.checks() {
        holds(Root::Check(c), t)?;
    }
    for (id, (_, t)) in &p.domains {
        holds(Root::Domain(*id), *t)?;
    }
    for (r, (_, t)) in &p.retags {
        holds(Root::Retag(*r), *t)?;
    }
    for (&(r, i), t) in &p.defaults {
        holds(Root::Default(r, i), *t)?;
    }
    for (&(r, i), (_, t)) in &p.computed {
        holds(Root::Computed(r, i), *t)?;
    }
    for (&(r, i, j), (_, f)) in &p.index_fields {
        holds(Root::IndexField(r, i, j), f.value)?;
    }
    for (i, a) in p.aggregates.iter().enumerate() {
        let Some(a) = a else { continue };
        for t in [a.init, a.step.1, a.merge.1, a.finish.1] {
            holds(Root::Aggregate(i), t)?;
        }
    }
    Ok(())
}
