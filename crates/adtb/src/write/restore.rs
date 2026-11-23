//! The write of a restore's rows without checks, and the one check of the whole database after.

use std::collections::BTreeMap;

use adtb_lang::kernel::Tm;
use adtb_lang::schema::Generator;
use adtb_lang::types::RelId;

use crate::btree::BTree;
use crate::exec::Val;
use crate::heap::Abandons;
use crate::plan::IndexId;
use crate::runtime::Runtime;
use crate::storage::Pid;

use super::admit::Admitted;
use super::eval::{Stmt, Violations, Violator, pairs, refuse, violate, violate_key};
use super::reindex::Scan;
use super::schema::Indexed;
use super::{Broken, Database, Error, Local, Rows, Stop, Text, Txn};

/// The database a restore fills.
#[derive(Debug)]
pub struct Restoring<'d, R: Runtime> {
    db: &'d Database<R>,
}

impl<R: Runtime> Database<R> {
    /// Returns the handle that writes rows into this database without checking them.
    ///
    /// Precondition: the caller is a restore, nothing else holds this database, and no reader
    /// may open it before [`Restoring::validate`] succeeds.
    pub(crate) fn admit_restored_rows(&self) -> Restoring<'_, R> {
        Restoring { db: self }
    }
}

impl<R: Runtime> Abandons for Restoring<'_, R> {}

impl<R: Runtime> Restoring<'_, R> {
    /// Write `rows` into their heaps as one committed transaction, checking only their sizes and
    /// entering no index entry.
    pub fn write(&self, rows: Rows) -> Result<(), Error> {
        let tx = self.db.begin()?;
        let written = self
            .db
            .admit_grant(
                crate::exec::Priority::AdHoc,
                (tx.began(), None),
                &Stop::default(),
            )
            .map(|grant| Stmt::reader(self.db, &tx, grant))
            .and_then(|stmt| {
                stmt.check_sizes(rows, Rows::new())
                    .and_then(|set| self.insert(&stmt, &tx, &set))
            });
        match written {
            Ok(()) => self.db.commit(tx).map(drop),
            Err(e) => Err(self.db.abandoned(tx, e)),
        }
    }

    fn insert(&self, stmt: &Stmt<'_, R>, tx: &Txn, set: &Admitted) -> Result<(), Error> {
        let db = self.db;
        let xid = tx.writer()?;
        for ((r, c), rows) in set.plus() {
            let heap = stmt.rel(*r).ctors[*c as usize].heap;
            let mut most = None;
            for row in rows {
                let stored = stmt.stored(heap, row)?;
                let h = &db.bindings.heaps()[&heap].heap;
                let mut judge = db.session.txns.judge_now(&db.pager, h.id().0);
                h.insert_pruning(&db.pager, xid, &stored, &mut judge)
                    .kept(self)?;
                if let Val::Int(k) = stmt.key_of(*r, row)? {
                    most = most.max(Some(k));
                }
            }
            let rel = stmt.rel(*r);
            if let Some(k) = most
                && rel.generate == Some(Generator::Sequence)
                && rel.single_key
            {
                db.advance_sequence(*r, k)?;
            }
        }
        Ok(())
    }

    /// Build every index of each of `rels` from its heaps, then check every key, unique,
    /// reference, domain, constraint and check once over the whole database.
    ///
    /// Fail with the violation of least name, as a statement does. A constraint reads the change
    /// as empty: `inserted` and `deleted` are empty and `prior` is the relation.
    pub fn validate(self, rels: &[RelId]) -> Result<(), Error> {
        let db = self.db;
        let mut duplicated = Vec::new();
        for r in rels {
            for (id, ix) in &db.rels[r].indexes {
                if let Some(name) = self.build(*r, *id, ix)? {
                    duplicated.push((*r, *id, ix, name));
                }
            }
        }
        let mut viol = Violations::new();
        let tx = db.begin_read(false)?;
        let checked = db
            .admit_grant(
                crate::exec::Priority::AdHoc,
                (tx.began(), None),
                &Stop::default(),
            )
            .map(|grant| Stmt::reader(db, &tx, grant))
            .and_then(|stmt| {
                duplicated
                    .iter()
                    .try_for_each(|&(r, id, ix, name)| {
                        duplicates(&stmt, r, (id, ix, name), &mut viol)
                    })
                    .and_then(|()| self.check(&stmt, rels, &mut viol))
            });
        db.abort_returning(tx, checked)?;
        refuse(viol)
    }

    /// Build index `id` of `rel` from its heaps into its tree. Returns the name of the key or
    /// unique it holds when two rows share one of its keys, the tree then built without that
    /// check.
    ///
    /// Precondition: the index's tree holds only its root.
    fn build<'i>(
        &self,
        rel: RelId,
        id: IndexId,
        ix: &'i Indexed,
    ) -> Result<Option<&'i str>, Error> {
        let db = self.db;
        let grant = db.admit_grant(
            crate::exec::Priority::Background,
            (db.rt.now(), None),
            &Stop::default(),
        )?;
        let scan = Scan::new(db, &db.rels[&rel], (id, ix), grant)?;
        let stop = || false;
        let owner = db.bindings.indexes()[&id].index.tree().owner();
        let build = |unique, made: &mut Vec<Pid>| {
            let tx = db.begin_read(false)?;
            let runs = scan.runs(None, Some(&tx));
            let runs = db.abort_returning(tx, runs)?;
            BTree::bulk(
                &db.pager,
                owner,
                scan.merged(runs, unique, &stop)?,
                made,
                &stop,
            )
        };
        let unique = ix.unique.as_deref();
        let mut made = Vec::new();
        let (built, duplicated) = match build(unique.is_some(), &mut made) {
            Ok(built) => (built, None),
            Err(Error::Internal(b)) if matches!(*b, Broken::DuplicateKey(_)) => {
                db.free_built(owner, &made);
                (build(false, &mut Vec::new())?, unique)
            }
            Err(e) => return Err(e),
        };
        let held = db.latches[&id].lock(&db.rt);
        held.index().take_over(&db.pager, built, None)?;
        Ok(duplicated)
    }

    /// Check the references and domains of every row of `rels`, and every constraint and check
    /// over the empty change.
    fn check(
        &self,
        stmt: &Stmt<'_, R>,
        rels: &[RelId],
        viol: &mut Violations,
    ) -> Result<(), Error> {
        let db = self.db;
        for r in rels {
            let mut rows = Rows::new();
            for (c, row) in stmt.rows(*r, None)? {
                rows.entry((*r, c)).or_default().insert(row);
            }
            stmt.check_rows(&rows, viol)?;
        }
        let schema = &*db.schema;
        let mut terms = db.normal.terms().clone();
        let named: Vec<(Violator, String, Tm)> = super::delta::denials(schema, &db.normal)?
            .into_iter()
            .map(|(of, name, t)| {
                let t = super::delta::unchanged(schema, &mut terms, t);
                (Violator::Denial(of), name, t)
            })
            .collect();
        for (of, name, t) in named {
            let text = Text {
                schema,
                terms: &terms,
                items: std::slice::from_ref(&t),
                params: &[],
                clock: 0,
                caller: None,
            };
            let denial = Stmt::new(
                db,
                stmt.tx,
                text,
                t,
                db.options,
                (Stop::default(), stmt.grant().clone()),
            )?;
            match denial.run(&[], &mut Local::default())?.result {
                Val::Set(report) => violate(viol, of, &name, report.iter().cloned()),
                other => return Err(Broken::DenialNotSet(other).into()),
            }
        }
        Ok(())
    }
}

/// Record the violation of unique index `id`, `ix`, of `rel`, holding the key or unique `name`:
/// each key two rows share for its key index, else each pair of rows that share one of its keys.
fn duplicates<R: Runtime>(
    stmt: &Stmt<'_, R>,
    rel: RelId,
    (id, ix, name): (IndexId, &Indexed, &str),
    viol: &mut Violations,
) -> Result<(), Error> {
    let r = stmt.rel(rel);
    let stored = &stmt.db.bindings.indexes()[&id];
    let of: Vec<u32> = stored.ctors.values().copied().collect();
    let types = stmt.store.key_types(id)?;
    let mut groups: BTreeMap<Vec<u8>, BTreeMap<Val, Val>> = BTreeMap::new();
    for (c, row) in stmt.rows(rel, Some(&of))? {
        let at = stmt.stored(r.ctors[c as usize].heap, &row)?;
        if let Some(key) = ix.keys.key(c, &at, &types)? {
            let group = groups
                .entry(stored.index.schema().encode(&key)?)
                .or_default();
            if id == r.key_index {
                group.insert(row.clone(), row);
            } else {
                group.insert(stmt.key_of(rel, &row)?, row);
            }
        }
    }
    groups.retain(|_, g| g.len() > 1);
    if id == r.key_index {
        for row in groups.values().filter_map(|g| g.values().next()) {
            violate_key(viol, name, rel, stmt.key_of(rel, row)?);
        }
    } else {
        violate(viol, Violator::Unique(id), name, pairs(groups.values()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Add to `out` each source file under `dir` that names the unchecked write's constructor, by its
    /// path under `root`, with the number of times it does.
    fn callers(root: &Path, dir: &Path, out: &mut Vec<(String, usize)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                callers(root, &path, out);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let n = text.matches(concat!("admit_restored_rows", "(")).count();
            if n > 0 {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                out.push((name.replace('\\', "/"), n));
            }
        }
    }

    #[test]
    fn only_a_restore_writes_rows_unchecked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        callers(&root, &root, &mut found);
        found.sort();
        let expected = [("api/dump.rs", 1), ("write/restore.rs", 1)];
        assert_eq!(expected.map(|(p, n)| (p.to_owned(), n)).to_vec(), found);
    }
}
