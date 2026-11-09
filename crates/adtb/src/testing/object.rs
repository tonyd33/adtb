use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use crate::btree::{BTree, Index};
use crate::heap::{Flags, Header, Heap, Version};
use crate::key::{CtorId, Entry, KeySchema, Tid, Value};
use crate::mvcc::{Status, StatusError, Xid};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::policy::Holder;
use crate::storage::{Pid, Quarantine};

use super::{Failed, Testing};

/// A structure of the database, named by the id the engine's catalog gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Object {
    /// The heap of a constructor.
    Heap(HeapId),
    /// The history heap of a constructor's heap.
    History(HeapId),
    /// The store of the large values of a constructor's heap.
    Large(HeapId),
    Index(IndexId),
    /// The history key index of the relation whose key index this is.
    HistoryIndex(IndexId),
    /// The tree recording when each transaction kept in history committed.
    CommitTimes,
    CommitLog,
    Catalog,
    /// The heap of the catalog's rows, which also holds the kept rows.
    CatalogRows,
    Sequences,
    Marks,
    DamageList,
}

/// The pages of the commit log at one instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitLogPages {
    root: Pid,
    directory: Vec<Pid>,
    status: BTreeMap<u64, Pid>,
}

impl CommitLogPages {
    pub fn root(&self) -> Pid {
        self.root
    }

    pub fn directory(&self) -> &[Pid] {
        &self.directory
    }

    /// Returns each status page by its number, the ids it holds starting at that number times
    /// the ids per page.
    pub fn status(&self) -> &BTreeMap<u64, Pid> {
        &self.status
    }
}

impl<'a, R: Runtime> Testing<'a, R> {
    /// Returns the heap of constructor `ctor` of relation `relation`, by qualified name.
    pub fn heap(self, relation: &str, ctor: &str) -> Option<Object> {
        self.db
            .tested_db()
            .heap_named(relation, ctor)
            .map(Object::Heap)
    }

    /// Returns the history heap of constructor `ctor` of relation `relation`, while history is
    /// kept.
    pub fn history(self, relation: &str, ctor: &str) -> Option<Object> {
        let w = self.db.tested_db();
        let id = w.heap_named(relation, ctor)?;
        w.history_heap_of(id).map(|_| Object::History(id))
    }

    /// Returns the store of the large values of the heap of constructor `ctor` of `relation`.
    pub fn large(self, relation: &str, ctor: &str) -> Option<Object> {
        self.db
            .tested_db()
            .heap_named(relation, ctor)
            .map(Object::Large)
    }

    /// Returns the index of relation `relation` of clause `clause`, as the schema writes it.
    pub fn index(self, relation: &str, clause: &str) -> Option<Object> {
        let w = self.db.tested_db();
        w.index_named(relation, clause).map(Object::Index)
    }

    /// Returns the history key index of relation `relation`, while history is kept.
    pub fn history_index(self, relation: &str) -> Option<Object> {
        let w = self.db.tested_db();
        let key = w.key_index_named(relation)?;
        w.history_index_of(key).map(|_| Object::HistoryIndex(key))
    }

    /// Returns every heap, history heap, large-value store, index and history key index of the
    /// database's relations.
    pub fn objects(self) -> Vec<Object> {
        let w = self.db.tested_db();
        let mut out = Vec::new();
        for id in w.heap_ids() {
            out.extend([Object::Heap(id), Object::Large(id)]);
            out.extend(w.history_heap_of(id).map(|_| Object::History(id)));
        }
        for id in w.index_ids() {
            out.push(Object::Index(id));
            out.extend(w.history_index_of(id).map(|_| Object::HistoryIndex(id)));
        }
        out
    }

    /// Returns the holder of the pages of `object`.
    pub fn holder(self, object: Object) -> Result<Holder, Failed> {
        let w = self.db.tested_db();
        let absent = Failed::Absent(object);
        match object {
            Object::Heap(id) | Object::History(id) => w.heap_holder(id).ok_or(absent),
            Object::Large(id) => w.heap_of(id).map(|_| Holder::LargeStore).ok_or(absent),
            Object::Index(id) | Object::HistoryIndex(id) => w.index_holder(id).ok_or(absent),
            Object::CommitTimes => Ok(Holder::CommitTimes),
            Object::CommitLog => Ok(Holder::CommitLog),
            Object::Catalog => Ok(Holder::CatalogRoot),
            Object::CatalogRows => Ok(Holder::CatalogRows),
            Object::Sequences => Ok(Holder::Counters),
            Object::Marks => Ok(Holder::MarksPage),
            Object::DamageList => Ok(Holder::DamageList),
        }
    }

    /// Returns the mark that names the heap or index `object` alone, as
    /// [`Plant::Mark`](super::Plant::Mark) sets it and admission of a call checks it. A history
    /// heap or index answers the mark of its relation's heap or key index. Fails with
    /// [`Failed::Shared`] for a large-value store or an engine structure, which no mark names
    /// apart from the database.
    pub fn own_mark(self, object: Object) -> Result<Quarantine, Failed> {
        let absent = Failed::Absent(object);
        match object {
            Object::Heap(id) | Object::History(id) => {
                let heap = self.db.tested_db().heap_of(id).ok_or(absent)?;
                Ok(Quarantine::Heap(heap.id().0))
            }
            Object::Index(id) | Object::HistoryIndex(id) => {
                let index = self.db.tested_db().index_of(id).ok_or(absent)?;
                Ok(Quarantine::Index(index.tree().owner()))
            }
            _ => Err(Failed::Shared(object)),
        }
    }

    /// Returns the owner the pages of `object` carry in their headers.
    pub fn owner(self, object: Object) -> Result<u32, Failed> {
        Ok(match object {
            Object::Heap(_) | Object::History(_) | Object::CatalogRows => {
                self.heap_object(object)?.id().0
            }
            Object::Large(_) => self.heap_object(object)?.large().id(),
            _ => self.tree(object)?.owner(),
        })
    }

    /// Returns the page `object` starts from: a heap's directory root, a store's or a tree's
    /// root, or the fixed page of an engine structure.
    pub fn root(self, object: Object) -> Result<Pid, Failed> {
        let pager = self.db.tested_pager();
        Ok(match object {
            Object::Heap(_) | Object::History(_) | Object::CatalogRows => {
                self.heap_object(object)?.root()
            }
            Object::Large(_) => self.heap_object(object)?.large().root(),
            Object::Index(_) | Object::HistoryIndex(_) | Object::CommitTimes => {
                self.tree(object)?.root()
            }
            Object::CommitLog => self.commit_log().root(),
            Object::Catalog => pager.catalog_root(),
            Object::Sequences => self.db.tested_db().sequences_root(),
            Object::Marks => pager.quarantine_page(),
            Object::DamageList => pager.damage_list_page(),
        })
    }

    /// Returns the pages of `object` its own listing names, in its order: a heap's or a store's
    /// data pages, a tree's pages from its root.
    pub fn pages(self, object: Object) -> Result<Vec<Pid>, Failed> {
        let pager = self.db.tested_pager();
        Ok(match object {
            Object::Heap(_) | Object::History(_) | Object::CatalogRows => {
                self.heap_object(object)?.pages(pager)?
            }
            Object::Large(_) => self.heap_object(object)?.large().pages(pager)?,
            Object::Index(_) | Object::HistoryIndex(_) | Object::CommitTimes => {
                self.tree(object)?.pages(pager)?
            }
            _ => return Err(Failed::Unlisted(object)),
        })
    }

    /// Returns every page `object` owns: for a heap its directory, data and visibility map
    /// pages; for a store its index and chunk pages.
    pub fn owned(self, object: Object) -> Result<Vec<Pid>, Failed> {
        let pager = self.db.tested_pager();
        Ok(match object {
            Object::Heap(_) | Object::History(_) | Object::CatalogRows => {
                self.heap_object(object)?.owned_pages(pager)?
            }
            Object::Large(_) => self.heap_object(object)?.large().owned_pages(pager)?,
            _ => return Err(Failed::Unlisted(object)),
        })
    }

    /// Returns the pages of tree `object` a walk from its root reaches, and whether the walk
    /// reached every page it names.
    pub fn reachable(self, object: Object) -> Result<(Vec<Pid>, bool), Failed> {
        Ok(self.tree(object)?.reachable(self.db.tested_pager()))
    }

    /// Returns the commit log's pages.
    pub fn commit_log(self) -> CommitLogPages {
        let clog = self.db.tested_db().txns().commit_log();
        CommitLogPages {
            root: clog.root(),
            directory: clog.directory_pages(),
            status: clog.status_pages().clone(),
        }
    }

    /// Returns what the commit log holds of transaction `x`.
    pub fn status(self, x: Xid) -> Result<Status, StatusError> {
        let w = self.db.tested_db();
        w.txns().recorded(self.db.tested_pager(), x)
    }

    /// Returns the constructors index `object` holds entries of.
    pub fn ctors(self, object: Object) -> Result<BTreeSet<CtorId>, Failed> {
        Ok(self.index_object(object)?.ctors().clone())
    }

    /// Returns how index `object` encodes its keys.
    pub fn key_schema(self, object: Object) -> Result<KeySchema, Failed> {
        Ok(self.index_object(object)?.schema().clone())
    }

    /// Returns the entries of index `object` of key `key`.
    pub fn entries(self, object: Object, key: &[Value]) -> Result<Vec<Entry>, Failed> {
        Ok(self
            .index_object(object)?
            .probe(self.db.tested_pager(), key)?)
    }

    /// Returns every entry of index `object`, in its order.
    pub fn all_entries(self, object: Object) -> Result<Vec<Entry>, Failed> {
        let index = self.index_object(object)?;
        Ok(index.range(self.db.tested_pager(), Bound::Unbounded, Bound::Unbounded)?)
    }

    /// Returns every version on `pages` of heap `object`, or on each of its pages when `None`,
    /// in page and then slot order. A page no longer of the heap is skipped, and refused while
    /// its directory still names it.
    pub fn versions(self, object: Object, pages: Option<&[Pid]>) -> Result<Vec<Version>, Failed> {
        let heap = self.heap_object(object)?;
        let pager = self.db.tested_pager();
        let listed;
        let pages = match pages {
            Some(p) => p,
            None => {
                listed = heap.pages(pager)?;
                &listed
            }
        };
        let mut out = Vec::new();
        heap.scan_pages(pager, pages, |v| out.push(v))?;
        Ok(out)
    }

    /// Returns the version at `tid` of heap `object`, `None` when it holds none there.
    pub fn read(self, object: Object, tid: Tid) -> Result<Option<Version>, Failed> {
        Ok(self
            .heap_object(object)?
            .read(self.db.tested_pager(), tid)?)
    }

    /// Returns the TID, header and flags of every version of heap `object`, in directory order
    /// and then slot order.
    pub fn headers(self, object: Object) -> Result<Vec<(Tid, Header, Flags)>, Failed> {
        Ok(self.heap_object(object)?.headers(self.db.tested_pager())?)
    }

    pub(super) fn heap_object(self, object: Object) -> Result<Arc<Heap>, Failed> {
        let w = self.db.tested_db();
        match object {
            Object::Heap(id) | Object::Large(id) => w.heap_of(id),
            Object::History(id) => w.history_heap_of(id),
            Object::CatalogRows => w.kept_store(),
            _ => None,
        }
        .ok_or(Failed::Absent(object))
    }

    pub(super) fn index_object(self, object: Object) -> Result<Index, Failed> {
        let w = self.db.tested_db();
        match object {
            Object::Index(id) => w.index_of(id),
            Object::HistoryIndex(id) => w.history_index_of(id),
            _ => None,
        }
        .ok_or(Failed::Absent(object))
    }

    pub(super) fn tree(self, object: Object) -> Result<BTree, Failed> {
        if object == Object::CommitTimes {
            let w = self.db.tested_db();
            return Ok(w.txns().history().ok_or(Failed::Absent(object))?.tree());
        }
        Ok(self.index_object(object)?.tree())
    }
}
