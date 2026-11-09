//! Every structure of a database file, each with the holder of its pages.

use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::schema::{ENGINE_MODULES, Schema};
use adtb_lang::types::RelId;

use crate::btree::{BTree, Index};
use crate::heap::{Heap, LargeStore};
use crate::key::CtorId;
use crate::mvcc::{CommitLog, Transactions};
use crate::runtime::Runtime;
use crate::storage::policy::{Holder, Phase};
use crate::storage::{PageError, Pager, Pid, Quarantine};
use crate::verify::{Inventory, Sum};

use super::history::key_indexes;
use super::ids::ctor_id;
use super::root::Root;
use super::{Catalog, roots_row};

/// Returns the holder of the pages of a heap or tree of relation `r`, named `object`.
pub fn relation_holder(schema: &Schema, r: RelId, object: Quarantine) -> Holder {
    let module = &schema.module(schema.relation(r).module).name;
    Holder::Relation {
        object,
        engine_module: ENGINE_MODULES.contains(&module.as_str()),
    }
}

/// Returns the holder of the pages of the index tree with owner `owner` of relation `r`.
pub fn index_holder(schema: &Schema, r: RelId, owner: u32) -> Holder {
    relation_holder(schema, r, Quarantine::Index(owner))
}

/// The heaps and indexes of one sum, each with its holder.
#[derive(Clone, Debug, Default)]
pub(super) struct Listed {
    heaps: Vec<(CtorId, Holder, Arc<Heap>)>,
    indexes: Vec<(Holder, Index)>,
}

impl Listed {
    /// Add `heap`, the heap of constructor `ctor`, held by `holder`.
    pub(super) fn heap(&mut self, ctor: CtorId, holder: Holder, heap: Arc<Heap>) {
        self.heaps.push((ctor, holder, heap));
    }

    /// Add `index`, held by `holder`.
    pub(super) fn index(&mut self, holder: Holder, index: Index) {
        self.indexes.push((holder, index));
    }
}

/// Every structure of a database file: the catalog's root, rows and counters, every heap and
/// index, current or history, the large-value stores they name and the commit times tree. The
/// commit log and the retired pages are read from the pager and transactions when asked.
#[derive(Clone, Debug)]
pub struct Structures {
    root: Pid,
    counters: Option<Pid>,
    rows: Option<Arc<Heap>>,
    sums: Vec<Listed>,
    trees: Vec<(Holder, BTree)>,
}

impl Catalog {
    /// Returns every structure of the file this catalog describes. A catalog that could not be
    /// read lists its root and counters pages alone.
    pub fn structures(&self) -> Structures {
        let mut out = Structures {
            root: self.root,
            counters: self.sequences(),
            rows: None,
            sums: Vec::new(),
            trees: Vec::new(),
        };
        if self.is_unread() {
            return out;
        }
        out.rows = Some(self.roots.clone());
        let schema = &self.schema;
        for &r in schema.ids.relations.keys() {
            let heaps = self
                .heaps
                .range((r, 0)..=(r, u32::MAX))
                .map(|(&(_, c), h)| {
                    let holder = relation_holder(schema, r, Quarantine::Heap(h.id().0));
                    (ctor_id(schema, r, c).stored(), holder, h.clone())
                })
                .collect();
            let indexes = self
                .indexes
                .values()
                .filter(|(spec, _, _)| spec.relation == r)
                .map(|(_, index, _)| {
                    let owner = index.tree().owner();
                    let holder = index_holder(schema, r, owner);
                    (holder, index.clone())
                })
                .collect();
            out.sums.push(Listed { heaps, indexes });
        }
        let keys = key_indexes(schema, &self.indexes);
        let (past, record) = self.kept.structures(schema, &self.heaps, &keys);
        out.sums.extend(past);
        out.trees.push((Holder::CommitTimes, record));
        out
    }
}

/// Returns the commit log's pages: its root, status and directory pages.
fn commit_log_pages(clog: &CommitLog) -> Vec<Pid> {
    let mut pages = vec![clog.root()];
    pages.extend(clog.status_pages().values());
    pages.extend(clog.directory_pages());
    pages
}

impl Structures {
    /// Returns the pages of the catalog's root, its rows heap and the commit log, as far as they
    /// can be read from the root page alone, for a catalog that does not open.
    pub fn unopened<R: Runtime>(pager: &Pager<R>) -> Unopened {
        let root = pager.catalog_root();
        let Ok(read) = pager.read(root).map(|page| Root::read(&page)) else {
            return Unopened {
                root,
                rows: Ok(Vec::new()),
                commit_log: Ok(Vec::new()),
            };
        };
        let large = LargeStore::open(read.large.0, read.large.1);
        let rows = Heap::open(pager, read.roots.0, read.roots.1, roots_row(), large)
            .map_or(Ok(Vec::new()), |heap| heap.owned_pages(pager));
        let commit_log = CommitLog::load(pager, read.commit_log).map(|c| commit_log_pages(&c));
        Unopened {
            root,
            rows,
            commit_log,
        }
    }

    /// Returns the page of the database's generated counters.
    pub fn counters(&self) -> Option<Pid> {
        self.counters
    }

    /// Returns the heap of the catalog's rows, which also holds the kept rows.
    pub fn rows(&self) -> Option<Arc<Heap>> {
        self.rows.clone()
    }

    /// Returns what holds the damaged page `pid` during `phase`, `None` when the mapping is
    /// incomplete and found no holder, as [`Pager::holders_meeting`] does. Add to `met` each page
    /// whose damage stopped a walk.
    pub fn holders<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        pid: Pid,
        phase: Phase,
        met: &mut BTreeSet<Pid>,
    ) -> Option<BTreeSet<Holder>> {
        let retired = pager.retired();
        let large = self.large();
        pager.holders_meeting(pid, phase, met, |h| {
            let clog = commit_log_pages(&txns.commit_log());
            h.walk(Holder::CatalogRoot, || Ok(vec![self.root]));
            h.walk(Holder::CommitLog, || Ok(clog.clone()));
            h.walk(Holder::Counters, || Ok(self.counters.into_iter().collect()));
            if let Some(rows) = &self.rows {
                h.walk(Holder::CatalogRows, || rows.held_pages(pager));
            }
            for sum in &self.sums {
                for (_, holder, heap) in &sum.heaps {
                    h.walk(*holder, || heap.held_pages(pager));
                }
            }
            for store in &large {
                h.walk(Holder::LargeStore, || store.held_pages(pager));
            }
            for sum in &self.sums {
                for (holder, index) in &sum.indexes {
                    h.walk(*holder, || index.tree().walk(pager));
                }
            }
            for (holder, tree) in &self.trees {
                h.walk(*holder, || tree.walk(pager));
            }
            h.walk(Holder::Retired, || Ok(retired.clone()));
        })
    }

    /// Returns the large-value store of every heap of a relation, current or history, once each.
    fn large(&self) -> Vec<LargeStore> {
        let mut out = Vec::new();
        for (_, _, heap) in self.sums.iter().flat_map(|s| &s.heaps) {
            if !out.contains(&heap.large()) {
                out.push(heap.large());
            }
        }
        out
    }

    /// Returns every structure for `verify`, with the commit log `clog`.
    pub fn inventory<'a>(&'a self, clog: &'a CommitLog) -> Inventory<'a> {
        let rows = self.rows.iter().map(|heap| Sum {
            heaps: vec![(CtorId(0), heap.as_ref())],
            indexes: Vec::new(),
        });
        let sums = self.sums.iter().map(|s| Sum {
            heaps: s.heaps.iter().map(|(c, _, h)| (*c, h.as_ref())).collect(),
            indexes: s.indexes.iter().map(|(_, i)| i).collect(),
        });
        Inventory {
            commit_log: Some(clog),
            sums: rows.chain(sums).collect(),
            trees: self.trees.iter().map(|(_, t)| *t).collect(),
            pages: std::iter::once(self.root).chain(self.counters).collect(),
        }
    }
}

/// The pages a catalog that does not open is read from, as far as its root page names them.
#[derive(Debug)]
pub struct Unopened {
    root: Pid,
    rows: Result<Vec<Pid>, PageError>,
    commit_log: Result<Vec<Pid>, PageError>,
}

impl Unopened {
    /// Returns what holds `pid`: the catalog's root, its rows heap or the commit log, else
    /// unreached. A listing stopped by a refusal of `pid` holds it.
    pub fn holder(&self, pid: Pid) -> Holder {
        let holds = |pages: &Result<Vec<Pid>, PageError>| match pages {
            Ok(pages) => pages.contains(&pid),
            Err(e) => e.pid == pid,
        };
        if pid == self.root {
            Holder::CatalogRoot
        } else if holds(&self.rows) {
            Holder::CatalogRows
        } else if holds(&self.commit_log) {
            Holder::CommitLog
        } else {
            Holder::Unreached
        }
    }
}
