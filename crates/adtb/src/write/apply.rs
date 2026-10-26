use std::collections::BTreeMap;
use std::convert::Infallible;

use adtb_lang::types::RelId;

use crate::exec::{Val, to_key, to_stored};
use crate::heap::{Flags, Heap, Link, walk};
use crate::key::{Entry, Tid, Value};
use crate::mvcc::{Probe, Status, Write, Xid, check_write, probe_unique};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::vacuum;

use super::eval::{Stmt, Violations, pairs, violate, violate_key};
use super::{Error, Rows};

/// A heap change a failed statement takes back.
#[derive(Clone, Copy, Debug)]
pub enum Undo {
    /// Delete the version the statement inserted.
    Kill(HeapId, Tid),
    /// Clear the deleter the statement gave the version.
    Restore(HeapId, Tid),
}

/// A version the statement wrote that needs entries in the indexes over its constructor.
struct Written {
    rel: RelId,
    ctor: u32,
    tid: Tid,
    row: Val,
}

impl<R: Runtime> Stmt<'_, R> {
    fn heap(&self, id: HeapId) -> &Heap {
        &self.db.heaps[&id].heap
    }

    fn status(&self, x: Xid) -> Result<Status, Infallible> {
        Ok(self
            .db
            .txns
            .status(&self.db.pager, x)
            .expect("the commit log of an open database reads"))
    }

    fn stored(&self, heap: HeapId, row: &Val) -> Vec<Value> {
        self.db.heaps[&heap]
            .row
            .iter()
            .zip(row.fields())
            .map(|(t, v)| to_stored(t, v))
            .collect()
    }

    /// Returns the TID of the version of `rel`'s row with key `key` the statement read, once no
    /// other running transaction writes it.
    fn writable(&self, rel: RelId, key: &Val) -> Result<(u32, Tid), Error> {
        let xid = self.tx.xid;
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
            let Ok(w) = check_write(xid, header.xmax, |x| self.status(x));
            match w {
                Write::Proceed => {
                    let id = self.heap(heap).id();
                    self.db
                        .ser
                        .lock(&self.db.rt)
                        .write_tuple(self.tx.sid, id, tid)?;
                    return Ok((c, tid));
                }
                Write::Wait(x) => self.db.waits.wait(Some(xid), x)?,
                Write::Fail(f) => return Err(f.into()),
            }
        }
    }

    fn inserted_into(&self, heap: HeapId) -> Result<(), Error> {
        let id = self.heap(heap).id();
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
                ops.entry((*r, self.key_of(*r, v))).or_default().0 = Some((*c, v.clone()));
            }
        }
        for ((r, c), rows) in plus {
            for v in rows {
                ops.entry((*r, self.key_of(*r, v)))
                    .or_default()
                    .1
                    .push((*c, v.clone()));
            }
        }
        let xid = self.tx.xid;
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
                    self.undo.borrow_mut().push(Undo::Restore(heap, tid));
                }
                (Some((c, _)), Some((d, n))) if c == d => {
                    let (c, tid) = self.writable(*rel, key)?;
                    let heap = r.ctors[c as usize].heap;
                    let indexed: Vec<usize> = r
                        .indexes
                        .iter()
                        .filter_map(|(_, ix)| ix.at.get(&c))
                        .flatten()
                        .copied()
                        .collect();
                    let row = self.stored(heap, n);
                    let up = vacuum::update(
                        pager,
                        &self.db.txns,
                        self.heap(heap),
                        tid,
                        xid,
                        &row,
                        &indexed,
                    )?;
                    self.undo
                        .borrow_mut()
                        .extend([Undo::Restore(heap, tid), Undo::Kill(heap, up.tid)]);
                    self.inserted_into(heap)?;
                    if !up.heap_only {
                        written.push(Written {
                            rel: *rel,
                            ctor: c,
                            tid: up.tid,
                            row: n.clone(),
                        });
                    }
                }
                (Some(_), Some((d, n))) => {
                    let (c, tid) = self.writable(*rel, key)?;
                    let to = r.ctors[*d as usize].heap;
                    let row = self.stored(to, n);
                    let from = r.ctors[c as usize].heap;
                    let at = self
                        .heap(from)
                        .retag(pager, tid, xid.0, self.heap(to), &row)?;
                    self.undo
                        .borrow_mut()
                        .extend([Undo::Restore(from, tid), Undo::Kill(to, at)]);
                    self.inserted_into(to)?;
                    written.push(Written {
                        rel: *rel,
                        ctor: *d,
                        tid: at,
                        row: n.clone(),
                    });
                }
                (None, Some((d, n))) => {
                    let heap = r.ctors[*d as usize].heap;
                    let row = self.stored(heap, n);
                    let tid = vacuum::insert(pager, &self.db.txns, self.heap(heap), xid, &row)?;
                    self.undo.borrow_mut().push(Undo::Kill(heap, tid));
                    self.inserted_into(heap)?;
                    written.push(Written {
                        rel: *rel,
                        ctor: *d,
                        tid,
                        row: n.clone(),
                    });
                }
            }
        }
        for ((r, _), rows) in plus {
            for v in rows {
                if let Val::Int(k) = self.key_of(*r, v) {
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
                Undo::Kill(heap, tid) => self.heap(heap).delete(pager, tid, self.tx.xid.0)?,
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
        let xid = self.tx.xid;
        let pager = &self.db.pager;
        for (id, ix) in &r.indexes {
            let Some(at) = ix.at.get(&w.ctor) else {
                continue;
            };
            let stored = &self.db.indexes[id];
            let types = self.store.key_types(*id);
            let fields = w.row.fields();
            let key: Vec<Value> = at
                .iter()
                .zip(&types)
                .map(|(i, t)| to_key(t, &fields[*i]))
                .collect();
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
                    for link in hot_run(pager, e.tid, self.heap(heap).id())? {
                        let Ok(p) = probe_unique(
                            xid,
                            &self.tx.snapshot,
                            link.header.xmin,
                            link.header.xmax,
                            |x| self.status(x),
                        );
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
                    violate_key(viol, name, w.rel, self.key_of(w.rel, &w.row));
                } else {
                    let group = groups.entry((w.rel, *id, encoded.clone())).or_default();
                    group.insert(self.key_of(w.rel, &w.row), w.row.clone());
                    for (c, heap, tid) in conflicts {
                        let v = self
                            .heap(heap)
                            .read(pager, tid)?
                            .expect("a conflicting version is there");
                        let row = self.row(w.rel, c, &v.row);
                        group.insert(self.key_of(w.rel, &row), row);
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

/// Returns the version at `tid` and its heap-only successors in heap `heap`: the versions an
/// index entry for `tid` stands for.
fn hot_run<R: Runtime>(
    pager: &crate::storage::Pager<R>,
    tid: Tid,
    heap: u32,
) -> Result<Vec<Link>, Error> {
    let mut links = Vec::new();
    walk(pager, tid, |l| {
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
