use std::collections::BTreeMap;

use adtb_lang::schema::Generator;
use adtb_lang::types::RelId;

use crate::exec::{StoredHeap, Val, to_stored};
use crate::heap::{self, Flags, Heap, Link, WriteTarget, walk};
use crate::key::{Entry, Tid, Value};
use crate::mvcc::{Probe, Status, Write, Xid, check_write, probe_unique};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::PageError;
use crate::vacuum;

use super::eval::{Stmt, Violations, pairs, violate, violate_key};
use super::{Error, Rejection, Rows};

/// Returns the write error of heap error `e`, met writing a row of constructor `ctor` of `rel`.
fn too_large(rel: RelId, ctor: u32) -> impl Fn(heap::Error) -> Error {
    move |e| match e {
        heap::Error::TooLarge { len, limit } => Error::Rejected(Rejection::RowTooLarge {
            relation: rel,
            ctor,
            size: len,
            limit,
        }),
        e => e.into(),
    }
}

/// A heap change a failed statement takes back.
#[derive(Clone, Copy, Debug)]
pub enum Undo {
    /// Delete the version the statement inserted.
    Kill(HeapId, Tid),
    /// Clear the deleter the statement gave the version.
    Restore(HeapId, WriteTarget),
}

/// A version the statement wrote that needs entries in the indexes over its constructor.
struct Written {
    rel: RelId,
    ctor: u32,
    tid: Tid,
    row: Val,
    /// The row as its heap stores it.
    stored: Vec<Value>,
}

impl<R: Runtime> Stmt<'_, R> {
    pub(super) fn check_sizes(&self, plus: &Rows) -> Result<(), Error> {
        for ((r, c), rows) in plus {
            let heap = self.rel(*r).ctors[*c as usize].heap;
            for row in rows {
                self.heap(heap)
                    .check_row(&self.stored(heap, row)?)
                    .map_err(too_large(*r, *c))?;
            }
        }
        for ((r, c), rows) in plus {
            let rel = self.rel(*r);
            let heap = rel.ctors[*c as usize].heap;
            for row in rows {
                let row = self.stored(heap, row)?;
                for (id, ix) in &rel.indexes {
                    let types = self.store.key_types(*id);
                    let Some(key) = ix.keys.key(*c, &row, &types)? else {
                        continue;
                    };
                    let stored = &self.db.indexes[id];
                    let ctor = *stored
                        .ctors
                        .iter()
                        .find(|(_, n)| **n == *c)
                        .expect("an index over a constructor holds its id")
                        .0;
                    if let Err(e) = stored
                        .index
                        .check_key(self.db.pager.page_size(), &key, ctor)
                    {
                        return Err(match e {
                            crate::btree::Error::TooLong { len, max } => {
                                let size = stored.index.schema().encode(&key).len();
                                Error::Rejected(Rejection::KeyTooLarge {
                                    relation: *r,
                                    ctor: *c,
                                    index: ix.clause.clone(),
                                    size,
                                    limit: max - (len - size),
                                })
                            }
                            e => e.into(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn heap(&self, id: HeapId) -> &Heap {
        &self.db.heaps[&id].heap
    }

    fn status(&self, x: Xid) -> Result<Status, PageError> {
        self.db.txns.status(&self.db.pager, x)
    }

    /// Returns `row` as `heap` stores it.
    pub(super) fn stored(&self, heap: HeapId, row: &Val) -> Result<Vec<Value>, Error> {
        Ok(self.db.heaps[&heap]
            .row
            .iter()
            .zip(row.fields()?)
            .map(|(t, v)| to_stored(t, v))
            .collect::<Result<_, _>>()?)
    }

    /// Returns the TID of the version of `rel`'s row with key `key` the statement read, once no
    /// other running transaction writes it.
    fn writable(&self, rel: RelId, key: &Val) -> Result<(u32, WriteTarget), Error> {
        let xid = self.tx.writer()?;
        loop {
            let (c, tid, _) = self
                .by_key(rel, key)?
                .expect("a row the statement deletes is in its snapshot");
            let heap = self.rel(rel).ctors[c as usize].heap;
            let header = self
                .heap(heap)
                .read(&self.db.pager, tid)?
                .expect("a version the statement read is there")
                .header;
            let w = check_write(xid, header.xmax, |x| self.status(x))?;
            match w {
                Write::Proceed => {
                    let id = self.heap(heap).id().0;
                    self.db
                        .ser
                        .lock(&self.db.rt)
                        .write_tuple(self.tx.sid, id, tid)?;
                    return Ok((c, WriteTarget::checked(tid, header)));
                }
                Write::Wait(x) => self.db.waits.wait(Some(xid), x)?,
                Write::Fail(f) => return Err(f.into()),
            }
        }
    }

    fn restorable(&self, heap: HeapId, tid: Tid, xid: Xid) -> Result<WriteTarget, Error> {
        let header = self
            .heap(heap)
            .read(&self.db.pager, tid)?
            .expect("a version the statement changed remains present")
            .header;
        assert_eq!(
            header.xmax, xid.0,
            "the statement owns the version it restores"
        );
        Ok(WriteTarget::checked(tid, header))
    }

    fn inserted_into(&self, heap: HeapId) -> Result<(), Error> {
        let id = self.heap(heap).id().0;
        self.db.ser.lock(&self.db.rt).insert_heap(self.tx.sid, id)?;
        Ok(())
    }

    /// Apply the normalised change set to the heaps and indexes, recording the key and unique
    /// conflicts its new versions meet.
    pub fn apply(&self, plus: &Rows, minus: &Rows, viol: &mut Violations) -> Result<(), Error> {
        type Pair = (Option<(u32, Val)>, Vec<(u32, Val)>);
        let mut ops: BTreeMap<(RelId, Val), Pair> = BTreeMap::new();
        for ((r, c), rows) in minus {
            for v in rows {
                ops.entry((*r, self.key_of(*r, v)?)).or_default().0 = Some((*c, v.clone()));
            }
        }
        for ((r, c), rows) in plus {
            for v in rows {
                ops.entry((*r, self.key_of(*r, v)?))
                    .or_default()
                    .1
                    .push((*c, v.clone()));
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let xid = self.tx.writer()?;
        let pager = &self.db.pager;
        let mut written = Vec::new();
        for ((rel, key), (old, news)) in &ops {
            let r = self.rel(*rel);
            match (old, news.first()) {
                (None, None) => {}
                (Some(_), None) => {
                    let (c, tid) = self.writable(*rel, key)?;
                    let heap = r.ctors[c as usize].heap;
                    self.heap(heap).delete(pager, tid, xid.0)?;
                    self.undo
                        .borrow_mut()
                        .push(Undo::Restore(heap, self.restorable(heap, tid.tid, xid)?));
                }
                (Some((c, _)), Some((d, n))) if c == d => {
                    let (c, tid) = self.writable(*rel, key)?;
                    let heap = r.ctors[c as usize].heap;
                    let indexed: Vec<usize> = r
                        .indexes
                        .iter()
                        .flat_map(|(_, ix)| ix.keys.reads(c))
                        .collect();
                    let row = self.stored(heap, n)?;
                    let up = vacuum::update(
                        pager,
                        &self.db.txns,
                        self.heap(heap),
                        tid,
                        xid,
                        &row,
                        &indexed,
                    )
                    .map_err(too_large(*rel, c))?;
                    self.undo.borrow_mut().extend([
                        Undo::Restore(heap, self.restorable(heap, tid.tid, xid)?),
                        Undo::Kill(heap, up.tid),
                    ]);
                    self.inserted_into(heap)?;
                    if !up.heap_only {
                        written.push(Written {
                            rel: *rel,
                            ctor: c,
                            tid: up.tid,
                            row: n.clone(),
                            stored: row,
                        });
                    }
                }
                (Some(_), Some((d, n))) => {
                    let (c, tid) = self.writable(*rel, key)?;
                    let to = r.ctors[*d as usize].heap;
                    let row = self.stored(to, n)?;
                    let from = r.ctors[c as usize].heap;
                    let at = self
                        .heap(from)
                        .retag(pager, tid, xid.0, self.heap(to), &row)
                        .map_err(too_large(*rel, *d))?;
                    self.undo.borrow_mut().extend([
                        Undo::Restore(from, self.restorable(from, tid.tid, xid)?),
                        Undo::Kill(to, at),
                    ]);
                    self.inserted_into(to)?;
                    written.push(Written {
                        rel: *rel,
                        ctor: *d,
                        tid: at,
                        row: n.clone(),
                        stored: row,
                    });
                }
                (None, Some((d, n))) => {
                    let heap = r.ctors[*d as usize].heap;
                    let row = self.stored(heap, n)?;
                    let tid = vacuum::insert(pager, &self.db.txns, self.heap(heap), xid, &row)
                        .map_err(too_large(*rel, *d))?;
                    self.undo.borrow_mut().push(Undo::Kill(heap, tid));
                    self.inserted_into(heap)?;
                    written.push(Written {
                        rel: *rel,
                        ctor: *d,
                        tid,
                        row: n.clone(),
                        stored: row,
                    });
                }
            }
        }
        for ((r, _), rows) in plus {
            let rel = self.rel(*r);
            if rel.generate != Some(Generator::Sequence) || !rel.single_key {
                continue;
            }
            for v in rows {
                if let Val::Int(k) = self.key_of(*r, v)? {
                    self.db.advance_sequence(*r, k)?;
                }
            }
        }
        let mut groups: BTreeMap<(RelId, IndexId, Vec<u8>), BTreeMap<Val, Val>> = BTreeMap::new();
        for w in &written {
            self.entries(w, viol, &mut groups)?;
        }
        for ((rel, ix, _), group) in &groups {
            let found = self.rel(*rel).indexes.iter().find(|(i, _)| i == ix);
            if let Some((_, indexed)) = found
                && let Some(name) = &indexed.name
            {
                violate(viol, name, pairs([group]));
            }
        }
        Ok(())
    }

    /// Take back every heap change the statement made, newest first. Its index entries stay,
    /// naming versions no snapshot sees.
    pub fn undo(&self) -> Result<(), Error> {
        let pager = &self.db.pager;
        for u in self.undo.borrow_mut().drain(..).rev() {
            match u {
                Undo::Kill(heap, tid) => {
                    let header = self
                        .heap(heap)
                        .read(pager, tid)?
                        .expect("a version the statement inserted remains present")
                        .header;
                    let xid = self.tx.writer()?;
                    assert_eq!(
                        header.xmin, xid.0,
                        "the statement owns the version it removes"
                    );
                    self.heap(heap)
                        .delete(pager, WriteTarget::checked(tid, header), xid.0)?;
                }
                Undo::Restore(heap, tid) => self.heap(heap).restore(pager, tid)?,
            }
        }
        Ok(())
    }

    /// Insert the entries of written version `w` into every index over its constructor,
    /// checking each unique against the latest committed state.
    fn entries(
        &self,
        w: &Written,
        viol: &mut Violations,
        groups: &mut BTreeMap<(RelId, IndexId, Vec<u8>), BTreeMap<Val, Val>>,
    ) -> Result<(), Error> {
        let r = self.rel(w.rel);
        let xid = self.tx.writer()?;
        let pager = &self.db.pager;
        for (id, ix) in &r.indexes {
            let types = self.store.key_types(*id);
            let Some(key) = ix.keys.key(w.ctor, &w.stored, &types)? else {
                continue;
            };
            let stored = &self.db.indexes[id];
            let ctor_id = *stored
                .ctors
                .iter()
                .find(|(_, c)| **c == w.ctor)
                .expect("an index over a constructor holds its id")
                .0;
            let encoded = stored.index.schema().encode(&key);
            let latch = loop {
                let latch = self.db.latches[id].lock(&self.db.rt);
                if !ix.desc.unique {
                    break latch;
                }
                let mut conflicts = Vec::new();
                let mut wait = None;
                for e in stored.index.probe(pager, &key)? {
                    let c = stored.ctors[&e.ctor];
                    let heap = r.ctors[c as usize].heap;
                    for link in hot_run(pager, e.tid, &self.db.heaps[&heap])? {
                        let p = probe_unique(
                            xid,
                            &self.tx.snapshot,
                            link.header.xmin,
                            link.header.xmax,
                            |x| self.status(x),
                        )?;
                        match p {
                            Probe::Skip => {}
                            Probe::Conflict => conflicts.push((c, heap, link.tid)),
                            Probe::Wait(x) => wait = Some(x),
                            Probe::Fail(f) => return Err(f.into()),
                        }
                    }
                }
                if let Some(x) = wait {
                    drop(latch);
                    self.db.waits.wait(Some(xid), x)?;
                    continue;
                }
                if conflicts.is_empty() {
                    break latch;
                }
                if ix.desc.id == r.key_index {
                    let name = ix.name.as_deref().expect("a key index has a name");
                    violate_key(viol, name, w.rel, self.key_of(w.rel, &w.row)?);
                } else {
                    let group = groups.entry((w.rel, *id, encoded.clone())).or_default();
                    group.insert(self.key_of(w.rel, &w.row)?, w.row.clone());
                    for (c, heap, tid) in conflicts {
                        let v = self
                            .heap(heap)
                            .read(pager, tid)?
                            .expect("a conflicting version is there");
                        let row = self.row(w.rel, c, &v.row);
                        group.insert(self.key_of(w.rel, &row)?, row);
                    }
                }
                break latch;
            };
            stored.index.insert(
                pager,
                &Entry {
                    key,
                    ctor: ctor_id,
                    tid: w.tid,
                },
            )?;
            self.db.ser.lock(&self.db.rt).insert_entry(
                self.tx.sid,
                stored.index.tree().root(),
                &encoded,
                ctor_id,
            )?;
            drop(latch);
        }
        Ok(())
    }
}

/// Returns the version at `tid` and its heap-only successors in heap `stored`: the versions an
/// index entry for `tid` stands for.
fn hot_run<R: Runtime>(
    pager: &crate::storage::Pager<R>,
    tid: Tid,
    stored: &StoredHeap,
) -> Result<Vec<Link>, Error> {
    let heap = stored.heap.id();
    let mut links = Vec::new();
    walk(pager, heap, &stored.sum, tid, |l| {
        let first = links.is_empty();
        if l.heap == heap && (first || l.flags.contains(Flags::HEAP_ONLY)) {
            links.push(*l);
            false
        } else {
            true
        }
    })?;
    Ok(links)
}
