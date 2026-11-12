use std::collections::BTreeMap;

use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::{Generator, OnDelete, ReferenceIndex, Schema};
use adtb_lang::textual::key_names;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::catalog::{IndexKeys, IndexSpec};
use crate::exec::types::ctors;
use crate::exec::{Conv, Routes, conv};
use crate::plan::{Catalog, HeapId, IndexDesc, IndexId, declared_elements};

use super::{Broken, Error, Rot};

/// A direct reference field of a constructor.
#[derive(Clone, Debug)]
pub struct RefField {
    pub name: String,
    /// Its position in a row of the constructor.
    pub at: usize,
    pub header: bool,
    pub target: RelId,
    pub ctors: Option<CtorSet>,
    /// The field is an `Option` of the key.
    pub optional: bool,
    pub action: OnDelete,
    /// An index leading with the field over every constructor holding it; `None` only for a
    /// relation variable of an engine module.
    pub index: Option<IndexId>,
}

/// A constructor of a relation variable's sum.
#[derive(Debug)]
pub struct Ctor {
    pub name: String,
    /// The header fields, then the constructor's own, with their types.
    pub fields: Vec<(String, Ty)>,
    pub heap: HeapId,
    pub convs: Vec<Conv>,
    pub refs: Vec<RefField>,
    /// The position of each key field, in name order.
    pub key_at: Vec<usize>,
}

/// An index of the catalog over a relation variable.
#[derive(Debug)]
pub struct Indexed {
    pub desc: IndexDesc,
    pub clause: String,
    pub keys: IndexKeys,
    /// The canonical name of the unique it holds, or of the key.
    pub name: Option<String>,
}

/// A declared unique of a relation variable.
#[derive(Debug)]
pub struct Unique {
    /// Its position among the relation's declared indexes.
    pub pos: usize,
    pub scope: Vec<u32>,
    /// The catalog index that holds it.
    pub index: IndexId,
    pub name: String,
}

/// What the engine derives from one relation variable's declaration.
#[derive(Debug)]
pub struct Rel {
    pub id: RelId,
    /// The name qualified by its module, as constraint names give it.
    pub qualified: String,
    pub name: String,
    pub elem: Ty,
    pub ctors: Vec<Ctor>,
    pub single_key: bool,
    pub key_index: IndexId,
    /// The key fields in the key index's order.
    pub key_fields: Vec<String>,
    pub generate: Option<Generator>,
    pub indexes: Vec<(IndexId, Indexed)>,
    pub uniques: Vec<Unique>,
}

pub type Rels = BTreeMap<RelId, Rel>;

impl Rel {
    pub fn all_ctors(&self) -> Vec<u32> {
        (0..self.ctors.len() as u32).collect()
    }

    /// Returns the heap of each constructor position of `ctors`, by position.
    pub fn routes(&self, ctors: impl IntoIterator<Item = u32>) -> Routes {
        ctors
            .into_iter()
            .map(|c| (c, self.ctors[c as usize].heap))
            .collect()
    }
}

/// Returns the field path an index field is written as, if it is one.
fn refs(schema: &Schema, rel: RelId, c: u32, indexes: &[IndexDesc]) -> Vec<RefField> {
    let ids = &schema.ids;
    schema
        .reference_fields(rel)
        .into_iter()
        .filter(|f| f.ctor.is_none_or(|own| own == c))
        .map(|f| {
            let index = match f.index {
                ReferenceIndex::Key => ids.key_id(rel),
                ReferenceIndex::Declared(i) => ids.index_id(rel, i),
                ReferenceIndex::Implied => ids.implied_id(rel, f.field_ref()),
            };
            RefField {
                at: f.at,
                header: f.ctor.is_none(),
                target: f.target,
                ctors: f.target_ctors,
                optional: f.optional,
                action: f.on_delete,
                index: index
                    .map(|id| IndexId(id.raw()))
                    .filter(|id| indexes.iter().any(|d| d.id == *id)),
                name: f.field,
            }
        })
        .collect()
}

fn qualified(schema: &Schema, rel: RelId) -> String {
    let r = schema.relation(rel);
    format!("{}::{}", schema.module(r.module).name, r.name)
}

/// Returns what the engine derives from every relation variable `catalog` describes.
pub fn rels(schema: &Schema, normal: &NormalProgram, catalog: &Catalog) -> Result<Rels, Error> {
    let mut out = Rels::new();
    let specs = crate::catalog::indexes(schema);
    for (rel, desc) in catalog.relations() {
        let r = schema.relation(*rel);
        let cs = ctors(schema, &r.elem).expect("a relation's element type is a sum");
        let def = schema.ty(r.elem_def);
        let key = key_names(schema, *rel);
        let ctor_list = cs
            .iter()
            .enumerate()
            .map(|(c, fields)| {
                let c = c as u32;
                let pos = |n: &str| fields.iter().position(|(f, _)| f == n);
                Ctor {
                    name: def.ctors()[c as usize].name.clone(),
                    heap: desc.heaps[&c],
                    convs: fields.iter().map(|(_, t)| conv(schema, t)).collect(),
                    refs: refs(schema, *rel, c, &desc.indexes),
                    key_at: key
                        .iter()
                        .map(|n| pos(n).expect("every constructor has the key fields"))
                        .collect(),
                    fields: fields.clone(),
                }
            })
            .collect::<Vec<_>>();
        let mut uniques = Vec::new();
        for (i, ix) in r.indexes.iter().enumerate() {
            if !ix.unique {
                continue;
            }
            let scope = ix
                .scope
                .clone()
                .unwrap_or_else(|| (0..cs.len() as u32).collect());
            let elements = declared_elements(i, ix);
            let index = desc
                .indexes
                .iter()
                .find(|d| d.unique && d.ctors == scope && d.elements == elements)
                .map(|d| d.id)
                .ok_or_else(|| Broken::UniqueUnindexed {
                    position: i,
                    relation: r.name.clone(),
                })?;
            uniques.push(Unique {
                pos: i,
                scope,
                index,
                name: schema.unique_name(*rel, i),
            });
        }
        let mut indexes = Vec::new();
        for d in &desc.indexes {
            let keys = IndexKeys::new(schema, Some(normal), *rel, &d.ctors, &d.elements)?;
            let mut name = if d.id == desc.key_index {
                Some(format!("{}.key", qualified(schema, *rel)))
            } else {
                uniques
                    .iter()
                    .find(|u| u.index == d.id)
                    .map(|u| u.name.clone())
            };
            let clause = match (specs.get(&d.id.0), d.fields()) {
                (Some(spec), _) => spec.clause.clone(),
                (None, Some(fields)) if d.unique => {
                    let fields = fields.into_iter().map(str::to_string).collect();
                    let clause = IndexSpec::key(schema, *rel, d.id.0, fields).clause;
                    name = Some(format!("{}.{clause}", qualified(schema, *rel)));
                    clause
                }
                (None, _) => panic!(
                    "index {} of relation {} is neither declared nor an old key",
                    d.id.0, r.name
                ),
            };
            let indexed = Indexed {
                desc: d.clone(),
                clause,
                keys,
                name,
            };
            indexes.push((d.id, indexed));
        }
        let key_desc = desc
            .indexes
            .iter()
            .find(|d| d.id == desc.key_index)
            .ok_or_else(|| Rot::NoKeyIndex(r.name.clone()))?;
        out.insert(
            *rel,
            Rel {
                id: *rel,
                qualified: qualified(schema, *rel),
                name: r.name.clone(),
                elem: r.elem.clone(),
                ctors: ctor_list,
                single_key: r.key.len() == 1,
                key_index: desc.key_index,
                key_fields: key_desc
                    .fields()
                    .ok_or_else(|| Broken::KeyNotFields(r.name.clone()))?
                    .into_iter()
                    .map(String::from)
                    .collect(),
                generate: r.generate,
                indexes,
                uniques,
            },
        );
    }
    Ok(out)
}
