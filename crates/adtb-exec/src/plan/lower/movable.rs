use std::collections::BTreeSet;
use std::rc::Rc;

use adtb_lang::kernel::{Kind, Prim, Tm, Var};

use super::{Planner, erase};
use crate::plan::Broken;

/// A term a plan may evaluate before its place, out of its order or in place of a read: one of
/// the conditions from the first of a list that are scalar and total, or a scalar and total term
/// built from parts of one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Moved(Tm);

impl Moved {
    /// Returns the term, to put in the plan at its new place.
    pub(super) fn place(self) -> Tm {
        self.0
    }
}

/// The position of a [`Moved`] condition in the [`Movable`] that handed it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Slot {
    list: u32,
    at: usize,
}

/// A list of conditions and its prefix that is scalar and total, which alone a plan may move
/// into builds, keys, index keys, unswitching or flattening.
pub(super) struct Movable {
    list: u32,
    conds: Vec<Tm>,
    prefix: usize,
}

impl Movable {
    /// Returns the condition at position `i` of the list, when it is in the prefix.
    pub(super) fn get(&self, i: usize) -> Option<Moved> {
        (i < self.prefix).then(|| Moved(self.conds[i]))
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (Slot, Moved)> + '_ {
        let list = self.list;
        self.conds[..self.prefix]
            .iter()
            .enumerate()
            .map(move |(at, t)| (Slot { list, at }, Moved(*t)))
    }

    /// Returns the list without the conditions at `moved`.
    pub(super) fn without(&self, moved: &[Slot]) -> Result<Vec<Tm>, Broken> {
        self.own(moved)?;
        Ok((0..self.conds.len())
            .filter(|i| !moved.iter().any(|s| s.at == *i))
            .map(|i| self.conds[i])
            .collect())
    }

    /// Returns the list with `t` in place of the condition at `at`.
    pub(super) fn with(&self, at: Slot, t: Tm) -> Result<Vec<Tm>, Broken> {
        self.own(&[at])?;
        let mut out = self.conds.clone();
        out[at.at] = t;
        Ok(out)
    }

    fn own(&self, slots: &[Slot]) -> Result<(), Broken> {
        match slots.iter().all(|s| s.list == self.list) {
            true => Ok(()),
            false => Err(Broken::ForeignSlot),
        }
    }
}

impl Planner<'_> {
    /// Returns `conds` with the conditions from the first of them that are scalar and total.
    pub(super) fn total_prefix(&mut self, conds: &[Tm]) -> Movable {
        let prefix = conds
            .iter()
            .take_while(|c| self.scalar(**c) && self.total(**c))
            .count();
        self.lists += 1;
        Movable {
            list: self.lists,
            conds: conds.to_vec(),
            prefix,
        }
    }

    /// Returns `t`, evaluated after every condition of `prefix`, as a term a plan may evaluate
    /// before them, when every condition of the list is in `prefix` and `t` is scalar and total.
    pub(super) fn after_prefix(&mut self, prefix: &Movable, t: Tm) -> Option<Moved> {
        let whole = prefix.prefix == prefix.conds.len();
        (whole && self.scalar(t) && self.total(t)).then_some(Moved(t))
    }

    /// Returns `t`, a part of `_of` or a term built from its parts, as a term a plan may move,
    /// when it is scalar and total.
    pub(super) fn part(&mut self, _of: &Moved, t: Tm) -> Option<Moved> {
        (self.scalar(t) && self.total(t)).then_some(Moved(t))
    }

    /// Returns the sides of equality `c` of two values of one type, as the side a build keys by
    /// and the side a probe looks up, when `fits` holds of them so.
    pub(super) fn equal_sides(
        &mut self,
        c: &Moved,
        mut fits: impl FnMut(&mut Self, &Moved, &Moved) -> bool,
    ) -> Option<(Moved, Moved)> {
        let Kind::Prim(Prim::Eq, args) = self.kind(c.0) else {
            return None;
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        [(a, b), (b, a)].into_iter().find_map(|(b, p)| {
            let (build, probe) = (self.part(c, b)?, self.part(c, p)?);
            fits(self, &build, &probe).then_some((build, probe))
        })
    }

    pub(super) fn moved_kind(&self, m: &Moved) -> Kind {
        self.kind(m.0)
    }

    pub(super) fn reads(&mut self, m: &Moved, vs: &[Var]) -> bool {
        self.mentions(m.0, vs)
    }

    /// Returns whether `m` reads a variable of `local`.
    pub(super) fn reads_any(&mut self, m: &Moved, local: &BTreeSet<Var>) -> bool {
        self.free_in(m.0, local)
    }

    pub(super) fn free_of(&mut self, m: &Moved) -> Rc<BTreeSet<Var>> {
        self.facts.free(&self.t, m.0)
    }
}
