use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::capability::RootKind;
use adtb_lang::schema::OnDelete;
use adtb_lang::types::RelId;

use crate::exec::{Held, Val};
use crate::runtime::Runtime;

use super::eval::{Arena, Eff, Effects, Env, Stmt};
use super::schema::RefField;
use super::{Error, Outcome, Rows};

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
    pub(super) fn boundary(&self, written: Effects, result: Val) -> Result<Outcome, Error> {
        let deleted = self.deleted(&written)?;
        let (plus, minus) = self.change_set(written, &deleted)?;
        self.settle(plus, minus, result)
    }

    /// Returns the rows the written effects delete, with every row a cascade deletes.
    fn deleted(&self, written: &Effects) -> Result<Deleted, Error> {
        let mut minus_written: Held<BTreeSet<(RelId, Val)>> = self.grant().held();
        let mut kept_keys: Held<BTreeSet<(RelId, Val)>> = self.grant().held();
        let mut removed: Held<BTreeSet<(RelId, Val)>> = self.grant().held();
        for e in written {
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
        let mut gone: Held<BTreeSet<(RelId, Val)>> = self.grant().held();
        for k in removed {
            if !kept_keys.contains(&k) {
                gone.insert(k)?;
            }
        }
        drop(kept_keys);
        let mut pre_rows: Held<BTreeMap<(RelId, Val), Val>> = self.grant().held();
        loop {
            let targets: BTreeSet<RelId> = gone.iter().map(|(r, _)| *r).collect();
            let mut more: Held<Vec<(RelId, Val)>> = self.grant().held();
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
    fn change_set(&self, written: Effects, deleted: &Deleted) -> Result<(Rows, Rows), Error> {
        let Deleted {
            minus_written,
            gone,
            pre_rows,
        } = deleted;
        let mut all = written;
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
                all.insert(Eff::Rep(r, u, new));
            }
        }
        let mut plus = Rows::new();
        let mut minus = Rows::new();
        for e in &all {
            match e {
                Eff::Ins(r, v) | Eff::Rep(r, _, v) => {
                    if !gone.contains(&(*r, self.key_of(*r, v)?)) {
                        add(&mut plus, *r, v.clone())?;
                    }
                }
                Eff::Del(..) => {}
            }
            if let Eff::Del(r, v) | Eff::Rep(r, v, _) = e {
                add(&mut minus, *r, v.clone())?;
            }
        }
        for (r, k) in gone.iter() {
            let row = match pre_rows.get(&(*r, k.clone())) {
                Some(u) => Some(u.clone()),
                None => self.by_key(*r, k)?.map(|(_, _, u)| u),
            };
            if let Some(u) = row {
                add(&mut minus, *r, u)?;
            }
        }
        normalise(&mut plus, &mut minus);
        Ok((plus, minus))
    }
}

pub(super) fn add(rows: &mut Rows, rel: RelId, v: Val) -> Result<(), Error> {
    rows.entry((rel, v.ctor_of()?)).or_default().insert(v);
    Ok(())
}

pub(super) fn normalise(plus: &mut Rows, minus: &mut Rows) {
    for (rc, p) in plus.iter_mut() {
        if let Some(m) = minus.get_mut(rc) {
            let both: Vec<Val> = p.intersection(m).cloned().collect();
            for v in both {
                p.remove(&v);
                m.remove(&v);
            }
        }
    }
    plus.retain(|_, rows| !rows.is_empty());
    minus.retain(|_, rows| !rows.is_empty());
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
