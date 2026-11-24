use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use adtb_lang::capability::RootKind;
use adtb_lang::schema::OnDelete;
use adtb_lang::types::RelId;

use crate::exec::{Grant, Held, Val};
use crate::runtime::Runtime;

use super::eval::{Arena, Eff, Effects, Env, Stmt};
use super::schema::RefField;
use super::{Counts, Error};

/// A row of a change set, by relation and constructor.
pub type Row = ((RelId, u32), Val);

/// The rows a statement inserts and deletes, held in a grant.
#[derive(Debug)]
pub struct ChangeSet {
    plus: Held<BTreeSet<Row>>,
    minus: Held<BTreeSet<Row>>,
}

impl ChangeSet {
    pub fn new(grant: &Arc<Grant>) -> ChangeSet {
        ChangeSet {
            plus: grant.held(),
            minus: grant.held(),
        }
    }

    /// Add row `v` of `rel` to the rows inserted.
    pub fn insert(&mut self, rel: RelId, v: Val) -> Result<(), Error> {
        self.plus.insert(((rel, v.ctor_of()?), v))?;
        Ok(())
    }

    /// Add row `v` of `rel` to the rows deleted.
    pub fn delete(&mut self, rel: RelId, v: Val) -> Result<(), Error> {
        self.minus.insert(((rel, v.ctor_of()?), v))?;
        Ok(())
    }

    /// Drop each row both inserted and deleted from both.
    pub fn normalise(&mut self) {
        let minus = &self.minus;
        let both = self.plus.split_off_where(|r| minus.contains(r));
        for r in both.iter() {
            self.minus.remove(r);
        }
    }

    pub fn plus(&self) -> &BTreeSet<Row> {
        &self.plus
    }

    pub fn minus(&self) -> &BTreeSet<Row> {
        &self.minus
    }

    /// Returns the rows of constructor `c` of `rel` that `rows` holds.
    pub fn of(rows: &BTreeSet<Row>, rel: RelId, c: u32) -> impl Iterator<Item = &Val> {
        let from = Bound::Included(((rel, c), Val::Int(i64::MIN)));
        let to = match c.checked_add(1) {
            Some(d) => Bound::Excluded(((rel, d), Val::Int(i64::MIN))),
            None => Bound::Unbounded,
        };
        rows.range((from, to)).map(|(_, v)| v)
    }

    /// Returns how many rows of each constructor it inserts, then deletes.
    pub fn counts(&self) -> (Counts, Counts) {
        let count = |rows: &BTreeSet<Row>| {
            let mut out = Counts::new();
            for (rc, _) in rows {
                *out.entry(*rc).or_default() += 1;
            }
            out
        };
        (count(&self.plus), count(&self.minus))
    }
}

/// The rows a statement's deletes remove.
struct Deleted {
    /// Each row a delete or replace removes.
    minus_written: Held<BTreeSet<(RelId, Val)>>,
    /// The key of each row that goes, cascades included.
    gone: Held<BTreeSet<(RelId, Val)>>,
    /// The row of each key a cascade removes.
    pre_rows: Held<BTreeMap<(RelId, Val), Val>>,
}

impl<R: Runtime> Stmt<'_, R> {
    /// Returns whether reference field `f` of `row` names a key in `gone`.
    fn names_gone(f: &RefField, row: &Val, gone: &BTreeSet<(RelId, Val)>) -> Result<bool, Error> {
        Ok(Self::names(f, row)?.is_some_and(|x| gone.contains(&(f.target, x))))
    }

    /// Derive the delete actions of the written effects, form the statement's change set,
    /// apply it, and check it at the statement boundary.
    pub(super) fn boundary(&self, written: Effects) -> Result<(Counts, Counts), Error> {
        let deleted = self.deleted(&written)?;
        let set = self.change_set(written, &deleted)?;
        drop(deleted);
        self.settle(set)
    }

    /// Returns the rows the written effects delete, with every row a cascade deletes.
    fn deleted(&self, written: &Effects) -> Result<Deleted, Error> {
        let mut minus_written: Held<BTreeSet<(RelId, Val)>> = self.rows_in().held();
        let mut kept_keys: Held<BTreeSet<(RelId, Val)>> = self.rows_in().held();
        let mut removed: Held<BTreeSet<(RelId, Val)>> = self.rows_in().held();
        for e in written.iter() {
            match e {
                Eff::Del(r, v) => {
                    minus_written.insert((*r, v.clone()))?;
                    removed.insert((*r, self.key_of(*r, v)?))?;
                }
                Eff::Rep(r, v, n) => {
                    minus_written.insert((*r, v.clone()))?;
                    kept_keys.insert((*r, self.key_of(*r, n)?))?;
                }
                Eff::Ins(r, n) => {
                    kept_keys.insert((*r, self.key_of(*r, n)?))?;
                }
            }
        }
        let mut gone: Held<BTreeSet<(RelId, Val)>> = self.rows_in().held();
        for k in removed {
            if !kept_keys.contains(&k) {
                gone.insert(k)?;
            }
        }
        drop(kept_keys);
        let mut pre_rows: Held<BTreeMap<(RelId, Val), Val>> = self.rows_in().held();
        loop {
            let targets: BTreeSet<RelId> = gone.iter().map(|(r, _)| *r).collect();
            let mut more: Held<Vec<(RelId, Val)>> = self.rows_in().held();
            for (r, fields) in self.referrers(&targets) {
                if !fields
                    .iter()
                    .flatten()
                    .any(|f| f.action == OnDelete::Cascade)
                {
                    continue;
                }
                for (_, (c, u)) in self.referring(r, &fields, &gone)? {
                    let k = (r, self.key_of(r, &u)?);
                    if minus_written.contains(&(r, u.clone())) || gone.contains(&k) {
                        continue;
                    }
                    let mut hit = false;
                    for f in &fields[c as usize] {
                        hit |= f.action == OnDelete::Cascade && Self::names_gone(f, &u, &gone)?;
                    }
                    if hit {
                        pre_rows.insert(k.clone(), u.clone())?;
                        more.push(k)?;
                    }
                }
            }
            if more.is_empty() {
                break;
            }
            for k in more {
                gone.insert(k)?;
            }
        }
        Ok(Deleted {
            minus_written,
            gone,
            pre_rows,
        })
    }

    /// Returns the rows the statement inserts and deletes: the written effects, the clears and
    /// retags of the rows that refer to a deleted one, and the deleted rows.
    fn change_set(&self, written: Effects, deleted: &Deleted) -> Result<ChangeSet, Error> {
        let Deleted {
            minus_written,
            gone,
            pre_rows,
        } = deleted;
        let mut set = ChangeSet::new(self.rows_in());
        for e in written {
            match e {
                Eff::Ins(r, v) => self.kept(&mut set, gone, r, v)?,
                Eff::Del(r, v) => set.delete(r, v)?,
                Eff::Rep(r, v, n) => {
                    set.delete(r, v)?;
                    self.kept(&mut set, gone, r, n)?;
                }
            }
        }
        let targets: BTreeSet<RelId> = gone.iter().map(|(r, _)| *r).collect();
        for (r, fields) in self.referrers(&targets) {
            for (_, (c, u)) in self.referring(r, &fields, gone)? {
                if minus_written.contains(&(r, u.clone()))
                    || gone.contains(&(r, self.key_of(r, &u)?))
                {
                    continue;
                }
                let mut reach: Vec<&RefField> = Vec::new();
                for f in fields[c as usize].iter().copied() {
                    if Self::names_gone(f, &u, gone)? {
                        reach.push(f);
                    }
                }
                let clears: Vec<&RefField> = reach
                    .iter()
                    .copied()
                    .filter(|f| f.action == OnDelete::Clear)
                    .collect();
                let retag = reach
                    .iter()
                    .any(|f| matches!(f.action, OnDelete::Retag(..)));
                let new = if retag {
                    let (var, body) = self.db.retags[&r];
                    let env: Env = [(var, u.clone())].into();
                    let v = self
                        .eval(Arena::Program(RootKind::DeleteAction), body, &env)?
                        .into_inner();
                    cleared(v, c, &clears)
                } else if !clears.is_empty() {
                    cleared(u.clone(), c, &clears)
                } else {
                    continue;
                };
                set.delete(r, u)?;
                self.kept(&mut set, gone, r, new)?;
            }
        }
        for (r, k) in gone.iter() {
            let row = match pre_rows.get(&(*r, k.clone())) {
                Some(u) => Some(u.clone()),
                None => self.by_key(*r, k)?.map(|(_, _, u)| u),
            };
            if let Some(u) = row {
                set.delete(*r, u)?;
            }
        }
        set.normalise();
        Ok(set)
    }

    /// Add row `v` of `rel` to the rows `set` inserts unless its key is in `gone`.
    fn kept(
        &self,
        set: &mut ChangeSet,
        gone: &BTreeSet<(RelId, Val)>,
        rel: RelId,
        v: Val,
    ) -> Result<(), Error> {
        if !gone.contains(&(rel, self.key_of(rel, &v)?)) {
            set.insert(rel, v)?;
        }
        Ok(())
    }
}

/// Returns `v`, the new value of a row of constructor `from`, with each field of `clears`, the
/// fields of `from`, set to `None` that `v` still has: every one when `v` is of `from`, else
/// only the header fields.
fn cleared(v: Val, from: u32, clears: &[&RefField]) -> Val {
    let Val::Ctor(c, fs) = v else {
        panic!("a row is a constructor value")
    };
    let mut fs = fs.to_vec();
    for f in clears.iter().filter(|f| c == from || f.header) {
        fs[f.at] = Val::none();
    }
    Val::ctor(c, fs)
}
