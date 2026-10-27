use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Kind, PathStep, Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};

use crate::exec::{self, Conv, Val, conv, from_stored, to_key};
use crate::key::{KeyType, Value};
use crate::plan::Element;

use super::layout;
use super::rows::{Step, at_path, path_steps};

/// How the rows of a relation's constructors are keyed in one index.
#[derive(Debug)]
pub struct IndexKeys {
    /// The parts of the key of a row, by constructor position.
    ctors: BTreeMap<u32, Ctor>,
}

#[derive(Debug)]
struct Ctor {
    parts: Vec<Part>,
    /// How each stored field of a row becomes a value an expression reads.
    convs: Vec<Conv>,
    /// The positions of the fields the key reads.
    reads: BTreeSet<usize>,
}

#[derive(Debug)]
enum Part {
    /// The value the steps reach, when every value on the way holds the constructor a step
    /// names.
    At(Vec<Step>),
    Expr(exec::Scalar),
}

impl IndexKeys {
    /// Returns the keying of an index of `rel` over `ctors` whose key is `elements`.
    ///
    /// Fails if the executor cannot compile an expression element.
    ///
    /// Panics if an element is derived and `normal`, the normal program of `schema`, is `None`.
    pub fn new(
        schema: &Schema,
        normal: Option<&NormalProgram>,
        rel: RelId,
        ctors: &[u32],
        elements: &[Element],
    ) -> Result<IndexKeys, exec::Error> {
        let mut out = BTreeMap::new();
        for &c in ctors {
            let row = layout::row(schema, rel, c);
            let mut parts = Vec::new();
            let mut reads = BTreeSet::new();
            for e in elements {
                match derived(normal, rel, e) {
                    Derived::Path(path) => {
                        let (steps, _) = path_steps(schema, rel, c, &path);
                        if let Some(Step::Field(i)) = steps.first() {
                            reads.insert(*i);
                        }
                        parts.push(Part::At(steps));
                    }
                    Derived::Expr(terms, x, body) => {
                        match fields_read(terms, body, x) {
                            Some(names) => reads.extend(
                                row.iter()
                                    .enumerate()
                                    .filter(|(_, (n, _))| names.contains(n))
                                    .map(|(i, _)| i),
                            ),
                            None => reads.extend(0..row.len()),
                        }
                        parts.push(Part::Expr(exec::Scalar::new(schema, terms, x, body)?));
                    }
                }
            }
            let convs = row.iter().map(|(_, t)| conv(schema, t)).collect();
            out.insert(
                c,
                Ctor {
                    parts,
                    convs,
                    reads,
                },
            );
        }
        Ok(IndexKeys { ctors: out })
    }

    /// Returns the key of stored row `row` of constructor `c`, its components of types `types`,
    /// or `None` when the index holds no entry for it: it is not over `c`, or a value on a path
    /// holds another constructor than the path names.
    ///
    /// Fails if an expression element faults on the row.
    pub fn key(
        &self,
        c: u32,
        row: &[Value],
        types: &[KeyType],
    ) -> Result<Option<Vec<Value>>, exec::Error> {
        let Some(ctor) = self.ctors.get(&c) else {
            return Ok(None);
        };
        let mut value = None;
        let mut out = Vec::with_capacity(ctor.parts.len());
        for (part, t) in ctor.parts.iter().zip(types) {
            match part {
                Part::At(steps) => match at_path(row, steps) {
                    Some(v) => out.push(v.clone()),
                    None => return Ok(None),
                },
                Part::Expr(scalar) => {
                    let v = value.get_or_insert_with(|| {
                        let fields = ctor.convs.iter().zip(row).map(|(c, v)| from_stored(c, v));
                        Val::ctor(c, fields.collect())
                    });
                    out.push(to_key(t, &scalar.eval(v.clone())?)?);
                }
            }
        }
        Ok(Some(out))
    }

    /// Returns the positions of the fields of a row of constructor `c` its key reads.
    pub fn reads(&self, c: u32) -> impl Iterator<Item = usize> + '_ {
        self.ctors
            .get(&c)
            .into_iter()
            .flat_map(|k| k.reads.iter().copied())
    }
}

/// Returns the type of each of `elements`, the key of an index of `rel` over a set holding
/// constructor `c`.
///
/// Panics if an element is derived and `normal`, the normal program of `schema`, is `None`.
pub fn element_types(
    schema: &Schema,
    normal: Option<&NormalProgram>,
    rel: RelId,
    c: u32,
    elements: &[Element],
) -> Vec<Ty> {
    elements
        .iter()
        .map(|e| match derived(normal, rel, e) {
            Derived::Path(path) => path_steps(schema, rel, c, &path).1,
            Derived::Expr(terms, _, body) => terms.ty(body).clone(),
        })
        .collect()
}

enum Derived<'n> {
    Path(Vec<PathStep>),
    Expr(&'n Terms, Var, Tm),
}

fn derived<'n>(normal: Option<&'n NormalProgram>, rel: RelId, e: &Element) -> Derived<'n> {
    match e {
        Element::Field(f) => Derived::Path(vec![PathStep::Field(f.clone())]),
        Element::Derived { index, element } => {
            let normal = normal.expect("a derived index element has the normal program");
            let (x, f) = &normal.index_fields[&(rel, *index, *element)];
            if f.path.is_empty() {
                Derived::Expr(&normal.terms, *x, f.value)
            } else {
                Derived::Path(f.path.clone())
            }
        }
    }
}

/// Returns the names of the fields of `x` that `t` reads, or `None` when it reads `x` otherwise
/// than by a field.
fn fields_read(terms: &Terms, t: Tm, x: Var) -> Option<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let mut todo = vec![t];
    while let Some(t) = todo.pop() {
        match terms.kind(t) {
            Kind::Field(row, f) if matches!(terms.kind(*row), Kind::Var(v) if *v == x) => {
                out.insert(f.clone());
            }
            Kind::Var(v) if *v == x => return None,
            _ => todo.extend(terms.children(t)),
        }
    }
    Some(out)
}
