use std::collections::BTreeMap;

use adtb_lang::schema::Generator;
use adtb_lang::types::RelId;

use crate::exec::{Val, to_stored};
use crate::heap::{self, Heap, Refused, Slots, Superseded, WriteTarget};
use crate::key::{Entry, Tid, Value};
use crate::mvcc::{Logged, Probe, Reader, Write, Xid};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::steps::Step;
use crate::storage::Pid;

use super::Error;
use super::admit::Admitted;
use super::eval::{Stmt, Violations, Violator, pairs, violate, violate_key};

/// Returns the write error of heap error `e`, met taking back a version's change.
fn undo_failure(e: heap::Error) -> Error {
    match e {
        heap::Error::TooLarge { .. } => {
            unreachable!("an undo rewrites a version's header in place: {e:?}")
        }
        e => e.into(),
    }
}

/// A heap change a failed statement takes back, with the heap it changed.
pub type Undo = (HeapId, heap::Undo);

/// The rows of a statement sharing a key of a unique index, by relation, index and encoded key,
/// with the unique's name.
type Groups = BTreeMap<(RelId, IndexId, Vec<u8>), (String, BTreeMap<Val, Val>)>;

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
    pub(super) fn heap(&self, id: HeapId) -> &Heap {
        &self.db.bindings.heaps()[&id].heap
    }

    fn reader_as(&self, me: Xid) -> Reader<Logged<R>> {
        self.db
            .session
            .txns
            .reader(&self.db.pager, self.tx.hold(), Some(me))
    }

    /// Returns `row` as `heap` stores it.
    pub(super) fn stored(&self, heap: HeapId, row: &Val) -> Result<Vec<Value>, Error> {
        Ok(self.db.bindings.heaps()[&heap]
            .row
            .iter()
            .zip(row.fields()?)
            .map(|(t, v)| to_stored(t, v))
            .collect::<Result<_, _>>()?)
    }

    /// Returns the TID of the version of `rel`'s row with key `key` the statement read, once no
    /// other running transaction writes it.
    ///
    /// Errors:
    /// - A refusal of the page of a version of `row`, a row of constructor `c` the statement read,
    ///   when its reads no longer see it.
    fn writable(
        &self,
        rel: RelId,
        key: &Val,
        c: u32,
        row: &Val,
    ) -> Result<(u32, WriteTarget), Error> {
        let xid = self.tx.writer()?;
        let reader = self.reader_as(xid);
        loop {
            let Some((c, tid, _)) = self.by_key(rel, key)? else {
                self.refuse_unseen(rel, key, c, row)?;
                continue;
            };
            let heap = self.heap(self.rel(rel).ctors[c as usize].heap);
            let Some(version) = heap.read(&self.db.pager, tid)? else {
                match heap.refuse_absent(&self.db.pager, tid) {
                    Some(e) => return Err(e.into()),
                    None => continue,
                }
            };
            let header = version.header;
            let checked = match reader.check_write(xid, &header.writers, heap.id().0) {
                Ok(checked) => checked,
                Err(e) => match version.refuse(&self.db.pager, e) {
                    Refused::Page(e) => return Err(e.into()),
                    Refused::Gone => continue,
                },
            };
            match checked {
                Write::Proceed => return Ok((c, WriteTarget::checked(tid, header))),
                Write::Wait(x) => self.wait(xid, x, Some(heap.id().0))?,
                Write::Fail(f) => return Err(f.into()),
            }
        }
    }

    /// Wait, as `xid` writing a version of `heap`, for transaction `on` to end, or fail with
    /// `Blocked` when the statement suspends instead. Fail with `Closed` once the engine closes,
    /// else with what stops the statement.
    fn wait(&self, xid: Xid, on: Xid, heap: Option<u32>) -> Result<(), Error> {
        if let Some(rerun) = self.stop.suspend {
            return Err(Error::Blocked { on, heap, rerun });
        }
        self.db
            .session
            .waits()
            .wait_pausing(xid, on, heap, || -> Result<(), Error> {
                self.db.rebuild_stopped().map_err(|_| Error::Closed)?;
                self.stopped()?;
                self.db.pause_a_poll();
                Ok(())
            })
    }

    /// Wait while an offline rebuild holds writes to any of `rels`, or fail with `Blocked` when
    /// the statement suspends instead. Fail with `Cancelled` once the engine closes.
    fn wait_for_rebuilds(&self, rels: impl Iterator<Item = RelId> + Clone) -> Result<(), Error> {
        let _waiting = self.db.waiting();
        while let Some(holder) = self.db.writes_held_by(rels.clone()) {
            self.db.rebuild_stopped()?;
            if let Some(rerun) = self.stop.suspend {
                return Err(Error::Blocked {
                    on: crate::mvcc::Gate::Offline.xid(),
                    heap: None,
                    rerun,
                });
            }
            self.stopped()?;
            self.db.pause_for_rebuild(holder);
        }
        Ok(())
    }

    /// Record the statement's write of the version `old` names against the SIREAD locks on it.
    fn superseded(&self, old: Superseded) -> Result<(), Error> {
        self.db.session.steps.at(Step::WriteChanged(old.change()));
        old.check(&mut self.db.session.ser.lock(&self.db.rt), self.tx.sid)?;
        self.db.session.steps.at(Step::WriteChecked);
        Ok(())
    }

    fn inserted_into(&self, heap: HeapId) -> Result<(), Error> {
        let id = self.heap(heap).id().0;
        self.db
            .session
            .ser
            .lock(&self.db.rt)
            .insert_heap(self.tx.sid, id)?;
        Ok(())
    }

    /// Apply the normalised change set to the heaps and indexes, recording the key and unique
    /// conflicts its new versions meet.
    ///
    /// Postconditions: the statement's reads see every row `set` inserts, two rows of one key
    /// included, and none it deletes.
    pub fn apply(&self, set: &Admitted, viol: &mut Violations) -> Result<(), Error> {
        type Pair = (Option<(u32, Val)>, Vec<(u32, Val)>);
        let (plus, minus) = (set.plus(), set.minus());
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
        self.wait_for_rebuilds(ops.keys().map(|(r, _)| *r))?;
        let xid = self.tx.writer()?;
        let pager = &self.db.pager;
        let mut written = Vec::new();
        for ((rel, key), (old, news)) in &ops {
            let r = self.rel(*rel);
            if old.is_none() && news.is_empty() {
                continue;
            }
            let target = |(c, row): &(u32, Val)| self.writable(*rel, key, *c, row);
            let mut write = || -> Result<(), Error> {
                match (old, news.first()) {
                    (None, None) => {}
                    (Some(old), None) => {
                        let (c, tid) = target(old)?;
                        let heap = r.ctors[c as usize].heap;
                        let old = (self.heap(heap).delete(pager, tid, xid)).record(
                            &mut self.undo.borrow_mut(),
                            heap,
                            heap,
                        )?;
                        self.superseded(old)?;
                    }
                    (Some(old @ (c, _)), Some((d, n))) if c == d => {
                        let (c, tid) = target(old)?;
                        let heap = r.ctors[c as usize].heap;
                        let indexed: Vec<usize> = r
                            .indexes
                            .iter()
                            .flat_map(|(_, ix)| ix.keys.reads(c))
                            .collect();
                        let row = self.stored(heap, n)?;
                        let h = self.heap(heap);
                        let mut judge = self.db.session.txns.judge_now(pager, h.id().0);
                        let (up, old) = h
                            .update_pruning(pager, tid, xid, &row, &indexed, &mut judge)
                            .record(&mut self.undo.borrow_mut(), heap, heap)?;
                        self.superseded(old)?;
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
                    (Some(old), Some((d, n))) => {
                        let (c, tid) = target(old)?;
                        let to = r.ctors[*d as usize].heap;
                        let row = self.stored(to, n)?;
                        let from = r.ctors[c as usize].heap;
                        let into = self.heap(to);
                        let mut judge = self.db.session.txns.judge_now(pager, into.id().0);
                        let (at, old) = self
                            .heap(from)
                            .retag_pruning(pager, tid, xid, into, &row, &mut judge)
                            .record(&mut self.undo.borrow_mut(), from, to)?;
                        self.superseded(old)?;
                        self.inserted_into(to)?;
                        written.push(Written {
                            rel: *rel,
                            ctor: *d,
                            tid: at,
                            row: n.clone(),
                            stored: row,
                        });
                    }
                    (None, Some((d, n))) => written.push(self.insert(*rel, *d, n, xid)?),
                }
                Ok(())
            };
            write()?;
            for (d, n) in news.iter().skip(1) {
                written.push(self.insert(*rel, *d, n, xid)?);
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
        let mut groups = Groups::new();
        for w in &written {
            self.entries(w, viol, &mut groups)?;
        }
        for ((_, ix, _), (name, group)) in &groups {
            violate(viol, Violator::Unique(*ix), name, pairs([group]));
        }
        Ok(())
    }

    /// Insert row `n` of constructor `d` of `rel` as a new version written by `xid`.
    fn insert(&self, rel: RelId, d: u32, n: &Val, xid: Xid) -> Result<Written, Error> {
        let heap = self.rel(rel).ctors[d as usize].heap;
        let row = self.stored(heap, n)?;
        let h = self.heap(heap);
        let mut judge = self.db.session.txns.judge_now(&self.db.pager, h.id().0);
        let tid = h
            .insert_pruning(&self.db.pager, xid, &row, &mut judge)
            .record(&mut self.undo.borrow_mut(), heap, heap)?;
        self.db.session.steps.at(Step::WriteInserted);
        self.inserted_into(heap)?;
        Ok(Written {
            rel,
            ctor: d,
            tid,
            row: n.clone(),
            stored: row,
        })
    }

    /// Take back every heap change the statement made, newest first. Its index entries stay,
    /// naming versions no snapshot sees.
    ///
    /// Errors:
    /// - A refusal of the page of a version the statement wrote that is not there as the
    ///   statement's.
    pub fn undo(&self) -> Result<(), Error> {
        for (heap, u) in self.undo.borrow_mut().drain(..).rev() {
            self.db.session.steps.at(Step::TakeBack);
            (self.heap(heap).take_back(&self.db.pager, u)).map_err(undo_failure)?;
        }
        Ok(())
    }

    /// Insert the entries of written version `w` into every index over its constructor,
    /// checking each unique against the latest committed state.
    fn entries(
        &self,
        w: &Written,
        viol: &mut Violations,
        groups: &mut Groups,
    ) -> Result<(), Error> {
        let r = self.rel(w.rel);
        let xid = self.tx.writer()?;
        let reader = self.reader_as(xid);
        let pager = &self.db.pager;
        let own = self.heap(r.ctors[w.ctor as usize].heap).id().0;
        for (id, ix) in &r.indexes {
            let types = self.store.key_types(*id)?;
            let Some(key) = ix.keys.key(w.ctor, &w.stored, &types)? else {
                continue;
            };
            let stored = &self.db.bindings.indexes()[id];
            let ctor_id = *stored
                .ctors
                .iter()
                .find(|(_, c)| **c == w.ctor)
                .expect("an index over a constructor holds its id")
                .0;
            let encoded = stored.index.schema().encode(&key);
            let latch = 'probe: loop {
                let latch = self.db.latches[id].lock(&self.db.rt);
                let Some(name) = &ix.unique else {
                    break latch;
                };
                let mut conflicts = Vec::new();
                let mut wait = None;
                for e in stored.index.probe(pager, &key)? {
                    // A rebuild's scan may have entered this version already.
                    if e.ctor == ctor_id && e.tid == w.tid {
                        continue;
                    }
                    let Some(&c) = stored.ctors.get(&e.ctor) else {
                        drop(latch);
                        return Err(stored.index.refuse_stray(pager, &e).into());
                    };
                    let heap = r.ctors[c as usize].heap;
                    let run = self.db.bindings.heaps()[&heap].heap.hot_run(pager, e.tid)?;
                    let at = (&*self.db.bindings.heaps()[&heap].heap, e.tid);
                    if run.is_empty() && heap::vacant(pager, at)?.is_some() {
                        drop(latch);
                        heap::refuse_named(pager, at, stored.index.tree().owner(), || {
                            let found = stored.index.probe(pager, &key)?;
                            Ok::<_, Error>(found.iter().any(|f| (f.ctor, f.tid) == (e.ctor, e.tid)))
                        })?;
                        continue 'probe;
                    }
                    for link in run {
                        let probe = match reader.probe_unique(xid, &link.header.writers, own) {
                            Ok(probe) => probe,
                            Err(e) => {
                                let at = Pid(link.tid.page());
                                let slots =
                                    Slots::Versions(self.db.bindings.heaps()[&heap].heap.id().0);
                                match Heap::refuse_unreserved(pager, (at, slots), latch, e) {
                                    Refused::Page(e) => return Err(e.into()),
                                    Refused::Gone => continue 'probe,
                                }
                            }
                        };
                        match probe {
                            Probe::Skip => {}
                            Probe::Conflict => conflicts.push((c, heap, link.tid)),
                            Probe::Wait(x) => wait = Some(x),
                            Probe::Fail(f) => return Err(f.into()),
                        }
                    }
                }
                if let Some(x) = wait {
                    drop(latch);
                    self.wait(xid, x, Some(own))?;
                    continue;
                }
                if conflicts.is_empty() {
                    break latch;
                }
                if ix.desc.id == r.key_index {
                    let key = self.key_of(w.rel, &w.row)?;
                    violate_key(viol, name, w.rel, key);
                } else {
                    let (_, group) = groups
                        .entry((w.rel, *id, encoded.clone()))
                        .or_insert_with(|| (name.clone(), BTreeMap::new()));
                    group.insert(self.key_of(w.rel, &w.row)?, w.row.clone());
                    for (c, heap, tid) in conflicts {
                        let v = self
                            .heap(heap)
                            .read(pager, tid)?
                            .expect("a conflicting version is there");
                        let row = self.row(w.rel, c, &v.row)?;
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
            self.db.session.ser.lock(&self.db.rt).insert_entry(
                self.tx.sid,
                stored.index.tree().root(),
                (&encoded, ctor_id),
                own,
            )?;
            drop(latch);
        }
        Ok(())
    }
}
