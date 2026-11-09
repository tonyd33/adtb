//! The heaps and trees of a database, each with what its damage quarantines.

use adtb_lang::schema::{ENGINE_MODULES, Schema};
use adtb_lang::types::RelId;

use crate::btree::BTree;
use crate::heap::{Heap, LargeStore};
use crate::runtime::Runtime;
use crate::storage::policy::Holder;
use crate::storage::{Holders, Pager, Pid, Quarantine};

/// Returns the holder of the pages of a heap or tree of relation `r`, named `object`.
pub fn relation_holder(schema: &Schema, r: RelId, object: Quarantine) -> Holder {
    let module = &schema.module(schema.relation(r).module).name;
    Holder::Relation {
        object,
        engine_module: ENGINE_MODULES.contains(&module.as_str()),
    }
}

/// What [`Holders`] walks to map a damaged page to its holder.
#[derive(Debug)]
pub struct Holdings<'a> {
    schema: &'a Schema,
    heaps: Vec<(Holder, &'a Heap)>,
    trees: Vec<(Holder, BTree)>,
    large: Vec<LargeStore>,
    retired: Vec<Pid>,
}

impl<'a> Holdings<'a> {
    pub fn new(schema: &'a Schema) -> Holdings<'a> {
        Holdings {
            schema,
            heaps: Vec::new(),
            trees: Vec::new(),
            large: Vec::new(),
            retired: Vec::new(),
        }
    }

    /// Add the heap of the catalog's rows.
    pub fn roots(&mut self, heap: &'a Heap) {
        self.heaps.push((Holder::CatalogRows, heap));
    }

    /// Add the heap of one constructor of `r` and its history heap.
    pub fn heaps(&mut self, r: RelId, heap: &'a Heap, past: Option<&'a Heap>) {
        let holder = relation_holder(self.schema, r, Quarantine::Heap(heap.id().0));
        self.heaps.push((holder, heap));
        self.heaps.extend(past.map(|p| (holder, p)));
    }

    /// Add `trees`, the current and history trees of the index of `r` with owner `owner`.
    pub fn trees(&mut self, r: RelId, owner: u32, trees: impl IntoIterator<Item = BTree>) {
        let holder = relation_holder(self.schema, r, Quarantine::Index(owner));
        self.trees.extend(trees.into_iter().map(|t| (holder, t)));
    }

    /// Add `pages`, of trees a swap replaced and no sweep has freed.
    pub fn retired(&mut self, pages: Vec<Pid>) {
        self.retired.extend(pages);
    }

    pub fn large(&mut self, store: LargeStore) {
        if !self.large.contains(&store) {
            self.large.push(store);
        }
    }

    /// Report the pages of every heap and tree to `h`.
    pub fn walk<R: Runtime>(&self, pager: &Pager<R>, h: &mut Holders) {
        for (holder, heap) in &self.heaps {
            h.walk(*holder, || heap.held_pages(pager));
        }
        for store in &self.large {
            h.walk(Holder::LargeStore, || store.held_pages(pager));
        }
        for (holder, tree) in &self.trees {
            h.walk(*holder, || tree.walk(pager));
        }
        h.walk(Holder::Retired, || Ok(self.retired.clone()));
    }
}
