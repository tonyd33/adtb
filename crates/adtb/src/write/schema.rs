use std::collections::BTreeMap;

use adtb_lang::kernel::{IndexField, Kind, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::{Generator, OnDelete, ReferenceIndex, Schema};
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::catalog::field_path;
use crate::exec::types::ctors;
use crate::exec::{Conv, conv};
use crate::plan::{Catalog, HeapId, IndexDesc, IndexId};

use super::Error;

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
    /// The position of each of its fields in a row, by constructor position.
    pub at: BTreeMap<u32, Vec<usize>>,
    /// The canonical name of the unique it holds, or of the key.
    pub name: Option<String>,
}

/// A declared unique of a relation variable.
#[derive(Debug)]
pub struct Unique {
    /// Its position among the relation's declared indexes.
    pub pos: usize,
    pub scope: Vec<u32>,
    /// Its field values, each a body over the row's variable.
    pub fields: Vec<(Var, IndexField)>,
    /// The catalog index that holds it, when there is one.
    pub index: Option<IndexId>,
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
    /// Returns the row type of constructor `c`.
    pub fn row_ty(&self, c: u32) -> Ty {
        match &self.elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: Some(vec![c]),
            },
            t => panic!("a relation's element type is a declared sum, found {t:?}"),
        }
    }

    pub fn all_ctors(&self) -> Vec<u32> {
        (0..self.ctors.len() as u32).collect()
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
                ReferenceIndex::Key => ids.keys.get(&rel),
                ReferenceIndex::Declared(i) => ids.indexes.get(&(rel, i)),
                ReferenceIndex::Implied => ids.implied.get(&(rel, f.ctor, f.field.clone())),
            };
            RefField {
                at: f.at,
                header: f.ctor.is_none(),
                target: f.target,
                ctors: f.target_ctors,
                optional: f.optional,
                action: f.on_delete,
                index: index
                    .map(|id| IndexId(*id))
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

fn unique_name(schema: &Schema, rel: RelId, i: usize) -> String {
    let name = qualified(schema, rel);
    let r = schema.relation(rel);
    let def = schema.ty(r.elem_def);
    let ix = &r.indexes[i];
    let paths: Option<Vec<String>> = ix.fields.iter().map(field_path).collect();
    let Some(paths) = paths else {
        let id = schema.ids.indexes[&(rel, i)];
        return format!("{name}.unique#{id}");
    };
    let all: CtorSet = (0..def.ctors().len() as u32).collect();
    let mut names: Vec<&str> = ix
        .scope
        .as_ref()
        .unwrap_or(&all)
        .iter()
        .map(|c| def.ctors()[*c as usize].name.as_str())
        .collect();
    names.sort_unstable();
    let set = match names.as_slice() {
        [one] => one.to_string(),
        many => format!("({})", many.join(" | ")),
    };
    format!("{name}.unique({}) in {set}", paths.join(", "))
}

/// Returns what the engine derives from every relation variable `catalog` describes.
pub fn rels(schema: &Schema, normal: &NormalProgram, catalog: &Catalog) -> Result<Rels, Error> {
    let mut out = Rels::new();
    for (rel, desc) in &catalog.relations {
        let r = schema.relation(*rel);
        let cs = ctors(schema, &r.elem).expect("a relation's element type is a sum");
        let def = schema.ty(r.elem_def);
        let mut key_names = r.key.clone();
        key_names.sort();
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
                    key_at: key_names
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
            let fields: Vec<(Var, IndexField)> = (0..ix.fields.len())
                .map(|k| normal.index_fields[&(*rel, i, k)].clone())
                .collect();
            let scope = ix
                .scope
                .clone()
                .unwrap_or_else(|| (0..cs.len() as u32).collect());
            let plain: Option<Vec<String>> = fields
                .iter()
                .map(|(_, f)| match normal.terms.kind(f.value) {
                    Kind::Field(x, name)
                        if f.path.is_empty() && matches!(normal.terms.kind(*x), Kind::Var(_)) =>
                    {
                        Some(name.clone())
                    }
                    _ => None,
                })
                .collect();
            let index = plain.and_then(|names| {
                desc.indexes
                    .iter()
                    .find(|d| d.unique && d.ctors == scope && d.fields == names)
                    .map(|d| d.id)
            });
            uniques.push(Unique {
                pos: i,
                scope,
                fields,
                index,
                name: unique_name(schema, *rel, i),
            });
        }
        let indexes =
            desc.indexes
                .iter()
                .map(|d| {
                    let at =
                        d.ctors
                            .iter()
                            .map(|c| {
                                let fields = &cs[*c as usize];
                                let ps =
                                    d.fields
                                        .iter()
                                        .map(|n| {
                                            fields.iter().position(|(f, _)| f == n).expect(
                                                "an index field is a field of its constructors",
                                            )
                                        })
                                        .collect();
                                (*c, ps)
                            })
                            .collect();
                    let name = if d.id == desc.key_index {
                        Some(format!("{}.key", qualified(schema, *rel)))
                    } else {
                        uniques
                            .iter()
                            .find(|u| u.index == Some(d.id))
                            .map(|u| u.name.clone())
                    };
                    (
                        d.id,
                        Indexed {
                            desc: d.clone(),
                            at,
                            name,
                        },
                    )
                })
                .collect();
        let key_desc = desc
            .indexes
            .iter()
            .find(|d| d.id == desc.key_index)
            .ok_or_else(|| Error::Corrupt(format!("relation {} has no key index", r.name)))?;
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
                key_fields: key_desc.fields.clone(),
                generate: r.generate,
                indexes,
                uniques,
            },
        );
    }
    Ok(out)
}
