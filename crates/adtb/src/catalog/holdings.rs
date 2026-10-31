//! The heaps and trees of a database, each with what its damage quarantines.

use adtb_lang::schema::{ENGINE_MODULES, Schema};
use adtb_lang::types::RelId;

use crate::btree::BTree;
use crate::heap::Heap;
use crate::runtime::Runtime;
use crate::storage::{Holders, Pager, Quarantine, Stopped};

/// What [`Holders`] walks to map a damaged page to its object. The damage of a relation of an
/// engine module quarantines the database.
#[derive(Debug)]
pub struct Holdings<'a> {
    schema: &'a Schema,
    heaps: Vec<(Option<Quarantine>, &'a Heap)>,
    trees: Vec<(Option<Quarantine>, BTree)>,
}

impl<'a> Holdings<'a> {
    pub fn new(schema: &'a Schema) -> Holdings<'a> {
        Holdings {
            schema,
            heaps: Vec::new(),
            trees: Vec::new(),
        }
    }

    /// Add the heap of the catalog's rows, whose damage quarantines `object`, nothing when `None`.
    pub fn roots(&mut self, heap: &'a Heap, object: Option<Quarantine>) {
        self.heaps.push((object, heap));
    }

    /// Add the heap of one constructor of `r` and its history heap.
    pub fn heaps(&mut self, r: RelId, heap: &'a Heap, past: Option<&'a Heap>) {
        let object = self.object(r, Quarantine::Heap(heap.id().0));
        self.heaps.push((object, heap));
        self.heaps.extend(past.map(|p| (object, p)));
    }

    /// Add `trees`, the current and history trees of the index of `r` with owner `owner`.
    pub fn trees(&mut self, r: RelId, owner: u32, trees: impl IntoIterator<Item = BTree>) {
        let object = self.object(r, Quarantine::Index(owner));
        self.trees.extend(trees.into_iter().map(|t| (object, t)));
    }

    fn object(&self, r: RelId, object: Quarantine) -> Option<Quarantine> {
        let module = &self.schema.module(self.schema.relation(r).module).name;
        Some(if ENGINE_MODULES.contains(&module.as_str()) {
            Quarantine::Database
        } else {
            object
        })
    }

    /// Report the pages of every heap and tree to `h`.
    pub fn walk<R: Runtime>(&self, pager: &Pager<R>, h: &mut Holders) {
        for (object, heap) in &self.heaps {
            h.walk(*object, || heap.owned_pages(pager).map_err(Stopped::from));
        }
        for (object, tree) in &self.trees {
            h.walk(*object, || tree.walk(pager));
        }
    }
}
