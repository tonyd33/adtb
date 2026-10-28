//! The stored holds of pending drops: a roots row for each pair of a pending-dropped object and a
//! bundle holding it, read at open and rewritten as holds are added and released.

use std::collections::BTreeSet;
use std::rc::Rc;

use adtb_lang::evolve::{Kept, Source};
use adtb_lang::parse::{Options, parse_migration};

use crate::heap::Heap;
use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::readers::{Pending, Readers, checked, without};
use super::{Catalog, Error, Migrating, ROOTS, delete_rows, int, reasons, uint};

/// The constructor of a roots row naming a pending-dropped constraint and a bundle holding it.
pub(super) const HELD_CONSTRAINT: u32 = 6;
/// The constructor of a roots row naming a pending-dropped check and a bundle holding it.
pub(super) const HELD_CHECK: u32 = 7;

/// Add the hold roots row `fields` of constructor `ctor`, one of [`HELD_CONSTRAINT`] and
/// [`HELD_CHECK`], names to `pending`.
pub(super) fn add(pending: &mut Vec<Pending>, ctor: u32, fields: &[Value]) -> Result<(), Error> {
    let [
        dropped,
        Value::Text(module),
        Value::Text(name),
        Value::Text(holder),
    ] = fields
    else {
        return Err(Error::Corrupt(format!(
            "a roots row of constructor {ctor} holds {fields:?}"
        )));
    };
    let check = ctor == HELD_CHECK;
    let dropped = uint(dropped)?;
    let same = |p: &&mut Pending| p.check == check && p.module == *module && p.name == *name;
    match pending.iter_mut().find(same) {
        Some(p) if p.dropped != dropped => {
            return Err(Error::Corrupt(format!(
                "{module}::{name} is held as dropped at versions {} and {dropped}",
                p.dropped
            )));
        }
        Some(p) => {
            p.blocking.insert(holder.clone());
        }
        None => pending.push(Pending {
            module: module.clone(),
            name: name.clone(),
            check,
            dropped,
            blocking: BTreeSet::from([holder.clone()]),
        }),
    }
    Ok(())
}

/// Returns the roots rows holding each drop of `pending` for each of its holders.
fn stored(pending: &[Pending]) -> Vec<Vec<Value>> {
    let mut out = Vec::new();
    for p in pending {
        let ctor = if p.check { HELD_CHECK } else { HELD_CONSTRAINT };
        for holder in &p.blocking {
            let fields = vec![
                int(p.dropped),
                Value::Text(p.module.clone()),
                Value::Text(p.name.clone()),
                Value::Text(holder.clone()),
            ];
            out.push(vec![Value::Sum { ctor, fields }]);
        }
    }
    out
}

/// Write as `m.x` the roots rows of `roots` that take the holds of `was` to those of `now`.
pub(super) fn rewrite<R: Runtime>(
    m: &Migrating<'_, R>,
    roots: &Heap,
    was: &[Pending],
    now: &[Pending],
) -> Result<(), Error> {
    let before = stored(was);
    let after = stored(now);
    let gone: Vec<Vec<Value>> = before
        .iter()
        .filter(|r| !after.contains(r))
        .cloned()
        .collect();
    delete_rows(m, roots, &gone)?;
    for row in after.iter().filter(|r| !before.contains(r)) {
        roots.insert(m.pager, m.x.0, row)?;
    }
    Ok(())
}

impl Readers {
    /// Returns the readers of a catalog at `source` as it opens: none live, the drops of
    /// `pending` held by the bundles stored with them, and the source that keeps every
    /// pending-dropped object.
    ///
    /// Fail with [`Error::Corrupt`] when a drop names a version the log does not hold, and with
    /// [`Error::Unaccepted`] when a migration of the log does not apply keeping the drops.
    pub(super) fn reopened(source: &Source, pending: Vec<Pending>) -> Result<Readers, Error> {
        let Some(first) = pending.iter().map(|p| p.dropped).min() else {
            return Ok(Readers::default());
        };
        let Some(start) = source.log.iter().find(|l| l.number == first) else {
            return Err(Error::Corrupt(format!(
                "a pending drop names version {first}, which the log does not hold"
            )));
        };
        let mut at = (*start.before).clone();
        for l in source.log.iter().filter(|l| l.number >= first) {
            let refused = |why: Vec<String>| Error::Unaccepted(l.number, why);
            let file = parse_migration(&l.text, Options::default())
                .ok()
                .map_err(|ds| refused(ds.into_iter().map(|d| d.message).collect()))?;
            let kept: Vec<Kept> = pending
                .iter()
                .filter(|p| p.dropped <= l.number)
                .map(Pending::kept)
                .collect();
            at = at
                .migrate_keeping(&file, &l.text, &kept)
                .map_err(|r| refused(reasons(r)))?;
        }
        let enforced = checked(at).map_err(|ds| {
            Error::Unchecked("the schema keeping the pending drops".to_string(), ds)
        })?;
        Ok(Readers {
            live: Default::default(),
            pending,
            enforced: Some(Rc::new(enforced)),
        })
    }
}

impl Catalog {
    /// Release the live reader of `bundle`. Every pending drop it was the last to hold takes
    /// effect. Returns once the release is durable.
    ///
    /// Fail with [`Error::Refused`] when no reader of `bundle` is live, and with
    /// [`Error::Unchecked`] when the schema without the drops that take effect does not check.
    pub fn release<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        bundle: &str,
    ) -> Result<(), Error> {
        if !self.readers.live.contains_key(bundle) {
            return Err(Error::Refused(vec![format!("bundle {bundle} is not live")]));
        }
        self.unhold(pager, txns, &BTreeSet::from([bundle.to_string()]))?;
        self.readers.live.remove(bundle);
        Ok(())
    }

    /// Release every hold of a pending drop by a bundle that is not a live reader. Every pending
    /// drop left with no holder takes effect. Returns once the release is durable.
    ///
    /// Fail with [`Error::Unchecked`] when the schema without the drops that take effect does
    /// not check.
    pub fn settle<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), Error> {
        let gone: BTreeSet<String> = self
            .readers
            .pending
            .iter()
            .flat_map(|p| &p.blocking)
            .filter(|b| !self.readers.live.contains_key(*b))
            .cloned()
            .collect();
        self.unhold(pager, txns, &gone)
    }

    /// Remove the bundles of `gone` from the holders of every pending drop, durably.
    fn unhold<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        gone: &BTreeSet<String>,
    ) -> Result<(), Error> {
        let (done, pending): (Vec<Pending>, Vec<Pending>) = self
            .readers
            .pending
            .iter()
            .cloned()
            .map(|mut p| {
                p.blocking.retain(|b| !gone.contains(b));
                p
            })
            .partition(|p| p.blocking.is_empty());
        if pending == self.readers.pending {
            return Ok(());
        }
        let enforced = match &self.readers.enforced {
            Some(e) if !pending.is_empty() && !done.is_empty() => {
                let source = without(&e.source, &done)?;
                let built = checked(source).map_err(|ds| {
                    Error::Unchecked("the schema without the released drops".to_string(), ds)
                })?;
                Some(Rc::new(built))
            }
            Some(e) if !pending.is_empty() => Some(Rc::clone(e)),
            _ => None,
        };
        let x = txns.begin(pager)?;
        let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let wrote = rewrite(&m, &self.roots, &self.readers.pending, &pending);
        txns.release(id);
        if let Err(e) = wrote {
            txns.abort(pager, x)?;
            return Err(e);
        }
        let lsn = txns.commit(pager, x)?;
        pager.wait_durable(lsn);
        self.readers.pending = pending;
        self.readers.enforced = enforced;
        Ok(())
    }
}
