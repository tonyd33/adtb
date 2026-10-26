use std::collections::BTreeMap;

use adtb_lang::types::{CtorSet, RelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HeapId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexId(pub u64);

/// The heaps and indexes of every relation variable a planned term may read.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub relations: BTreeMap<RelId, RelationDesc>,
}

#[derive(Clone, Debug)]
pub struct RelationDesc {
    /// The heap of each constructor of the relation's sum.
    pub heaps: BTreeMap<u32, HeapId>,
    /// The index over every constructor whose fields are the relation's key fields.
    pub key_index: IndexId,
    /// Every index of the relation, the key index included.
    pub indexes: Vec<IndexDesc>,
}

#[derive(Clone, Debug)]
pub struct IndexDesc {
    pub id: IndexId,
    pub ctors: CtorSet,
    /// Fields every constructor of `ctors` has, in key order.
    pub fields: Vec<String>,
    pub unique: bool,
    /// Whether a probe may fix a proper prefix of the fields.
    pub ordered: bool,
}

impl Catalog {
    pub fn relation(&self, r: RelId) -> &RelationDesc {
        self.relations
            .get(&r)
            .unwrap_or_else(|| panic!("the catalog describes relation {r:?}"))
    }

    pub fn heap(&self, r: RelId, ctor: u32) -> HeapId {
        *self
            .relation(r)
            .heaps
            .get(&ctor)
            .unwrap_or_else(|| panic!("relation {r:?} has a heap for constructor {ctor}"))
    }
}
