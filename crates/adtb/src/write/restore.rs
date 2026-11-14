//! The write of a restore's rows without checks, and the one check of the whole database after.

use std::collections::BTreeMap;

use adtb_lang::kernel::{Kind, Tm, Transition};
use adtb_lang::schema::Generator;
use adtb_lang::types::RelId;

use crate::btree::BTree;
use crate::btree::build::{BuildError, merge_runs};
use crate::exec::Val;
use crate::plan::IndexId;
use crate::runtime::Runtime;

use super::apply::too_large;
use super::eval::{Arena, Env, Stmt, Violations, pairs, refuse, violate, violate_key};
use super::reindex::Scan;
use super::schema::Indexed;
use super::{Broken, Database, Error, Rows, Stop, Text, Txn};

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

impl<R: Runtime> Restoring<'_, R> {
    /// Write `rows` into their heaps as one committed transaction, checking only their sizes and
    /// entering no index entry.
    pub fn write(&self, rows: &Rows) -> Result<(), Error> {
        let tx = self.db.begin()?;
        match self.insert(&tx, rows) {
            Ok(()) => self.db.commit(tx).map(drop),
            Err(e) => Err(self.db.abandoned(tx, e)),
        }
    }

    fn insert(&self, tx: &Txn, rows: &Rows) -> Result<(), Error> {
        let db = self.db;
        let stmt = Stmt::reader(db, tx);
        stmt.check_sizes(rows)?;
        let xid = tx.writer()?;
        for ((r, c), rows) in rows {
            let heap = stmt.rel(*r).ctors[*c as usize].heap;
            let mut most = None;
            for row in rows {
                let stored = stmt.stored(heap, row)?;
                let h = &db.bindings.heaps()[&heap].heap;
                let mut judge = db.session.txns.judge_now(&db.pager, h.id().0);
                h.insert_pruning(&db.pager, xid, &stored, &mut judge)
                    .map_err(too_large(*r, *c))?;
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
                if self.build(*r, *id, ix)? {
                    duplicated.push((*r, *id, ix));
                }
            }
        }
        let mut viol = Violations::new();
        let tx = db.begin_read(false)?;
        let stmt = Stmt::reader(db, &tx);
        let checked = duplicated
            .iter()
            .try_for_each(|&(r, id, ix)| duplicates(&stmt, r, id, ix, &mut viol))
            .and_then(|()| self.check(&stmt, rels, &mut viol));
        drop(stmt);
        db.abort_returning(tx, checked)?;
        refuse(viol)
    }

    /// Build index `id` of `rel` from its heaps into its tree. Returns whether it is unique and
    /// two rows share one of its keys, the tree then built without that check.
    ///
    /// Precondition: the index's tree holds only its root.
    fn build(&self, rel: RelId, id: IndexId, ix: &Indexed) -> Result<bool, Error> {
        let db = self.db;
        let scan = Scan {
            db,
            rel: &db.rels[&rel],
            id,
            ix,
        };
        let runs = || {
            let tx = db.begin_read(false)?;
            let runs = scan.runs(None, Some(&tx));
            db.abort_returning(tx, runs)
        };
        let stop = || false;
        let (entries, duplicated) = match merge_runs(runs()?, ix.desc.unique, &stop) {
            Ok(entries) => (entries, false),
            Err(BuildError::Duplicate(_)) => (merge_runs(runs()?, false, &stop)?, true),
            Err(e) => return Err(e.into()),
        };
        let tree = db.bindings.indexes()[&id].index.tree();
        let built = BTree::bulk(&db.pager, tree.owner(), entries, &mut Vec::new(), &stop)?;
        tree.take_over(&db.pager, built, None)?;
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
        let mut terms = db.normal.terms.clone();
        for node in &mut terms.nodes {
            if let Kind::Transition(x, r, s) = &node.kind {
                node.kind = match x {
                    Transition::Prior => Kind::Rel(*r, s.clone()),
                    Transition::Inserted | Transition::Deleted => Kind::EmptySet,
                };
            }
        }
        let schema = &*db.schema;
        let named = super::delta::denials(schema, &db.normal)?;
        let text = Text {
            schema,
            terms: &terms,
            items: &[],
            params: &[],
            clock: 0,
            caller: None,
        };
        let empty = Stmt::new(db, stmt.tx, text, Tm(0), db.options, Stop::default());
        for (name, t) in named {
            match empty.eval(Arena::Statement, t, &Env::new())? {
                Val::Set(report) => violate(viol, &name, report.iter().cloned()),
                other => return Err(Broken::DenialNotSet(other).into()),
            }
        }
        Ok(())
    }
}

/// Record the violation of unique index `id`, `ix`, of `rel`: each key two rows share for its
/// key index, else each pair of rows that share one of its keys.
fn duplicates<R: Runtime>(
    stmt: &Stmt<'_, R>,
    rel: RelId,
    id: IndexId,
    ix: &Indexed,
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
                .entry(stored.index.schema().encode(&key))
                .or_default();
            if id == r.key_index {
                group.insert(row.clone(), row);
            } else {
                group.insert(stmt.key_of(rel, &row)?, row);
            }
        }
    }
    groups.retain(|_, g| g.len() > 1);
    let Some(name) = ix.name.as_deref() else {
        let key = groups.into_keys().next().unwrap_or_default();
        return Err(Broken::DuplicateKey(key).into());
    };
    if id == r.key_index {
        for row in groups.values().filter_map(|g| g.values().next()) {
            violate_key(viol, name, rel, stmt.key_of(rel, row)?);
        }
    } else {
        violate(viol, name, pairs(groups.values()));
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
