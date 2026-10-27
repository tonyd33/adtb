use std::collections::BTreeMap;

use adtb_lang::ast::{ExprKind, QName};
use adtb_lang::schema::IndexDef;
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
    /// The elements of its key, in key order.
    pub elements: Vec<Element>,
    pub unique: bool,
    /// Whether a probe may fix a proper prefix of the elements.
    pub ordered: bool,
}

/// One element of an index's key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Element {
    /// A field every constructor of the index has.
    Field(String),
    /// Element `element` of the relation's declared index `index`: a path into a value-level sum
    /// field, or an expression of the row.
    Derived { index: usize, element: usize },
}

impl IndexDesc {
    /// Returns the fields of an index whose every element is a field.
    pub fn fields(&self) -> Option<Vec<&str>> {
        self.elements
            .iter()
            .map(|e| match e {
                Element::Field(f) => Some(f.as_str()),
                Element::Derived { .. } => None,
            })
            .collect()
    }
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

/// Returns the key elements of `def`, the declared index `i` of its relation.
pub fn declared_elements(i: usize, def: &IndexDef) -> Vec<Element> {
    def.fields
        .iter()
        .enumerate()
        .map(|(k, e)| match &e.kind {
            ExprKind::Name(QName { module: None, name }) => Element::Field(name.text.clone()),
            _ => Element::Derived {
                index: i,
                element: k,
            },
        })
        .collect()
}
