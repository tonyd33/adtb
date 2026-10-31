use std::collections::{BTreeMap, BTreeSet};

use crate::check::{Adhoc, Program, Query, Txn};
use crate::kernel::{Terms, Tm, Var};
use crate::norm::NormalProgram;
use crate::schema::Schema;
use crate::types::*;

mod aggregate;
mod coverage;
mod eval;
mod grants;
mod names;
mod shape;
mod value;
mod write;

#[cfg(test)]
mod tests;

pub use coverage::{EffectiveGrant, EffectiveGrants, Mode, Right};
pub use names::Nameable;
use value::{Env, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConstructorId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub relation: RelationId,
    pub constructor: ConstructorId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Requirement {
    ReadConstructor(Target),
    ReadField(Target, FieldId),
    Insert(Target),
    Delete(Target),
    UpdateField(Target, FieldId),
    Retag {
        relation: RelationId,
        from: ConstructorId,
        to: ConstructorId,
    },
    ReadFiles,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requirements {
    required: BTreeSet<Requirement>,
    keys: BTreeSet<(Target, FieldId)>,
}

/// The target every row of a temp relation stands for. A temp relation has no identity in the
/// schema, and its text alone reads and writes it.
const TEMP_TARGET: Target = Target {
    relation: RelationId(u64::MAX),
    constructor: ConstructorId(u64::MAX),
};

fn is_temp(schema: &Schema, rel: RelId) -> bool {
    schema.module_named(crate::check::TEMP_MODULE) == Some(schema.relation(rel).module)
}

impl Requirements {
    pub fn iter(&self) -> impl Iterator<Item = &Requirement> {
        self.required.iter()
    }

    /// Returns the requirements of reading every field of every constructor of `relations`.
    pub fn whole(
        schema: &Schema,
        relations: impl IntoIterator<Item = RelId>,
    ) -> Result<Requirements, Error> {
        use crate::ids::Slot;
        let mut out = Requirements::default();
        for rel in relations {
            let def = schema.relation(rel).elem_def;
            let relation = RelationId(
                *schema
                    .ids
                    .relations
                    .get(&rel)
                    .ok_or(Error::MissingIdentity(Slot::Relation(rel)))?,
            );
            let header: Vec<&str> = schema
                .ty(def)
                .header()
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            for (at, ctor) in schema.ty(def).ctors().iter().enumerate() {
                let at = at as u32;
                let target = Target {
                    relation,
                    constructor: ConstructorId(
                        *schema
                            .ids
                            .ctors
                            .get(&(def, at))
                            .ok_or(Error::MissingIdentity(Slot::Ctor(def, at)))?,
                    ),
                };
                out.required.insert(Requirement::ReadConstructor(target));
                let own = ctor.fields.iter().map(|f| f.name.as_str());
                for name in header.iter().copied().chain(own) {
                    let scope = (!header.contains(&name)).then_some(at);
                    let id = schema
                        .ids
                        .fields
                        .get(&(def, scope, name.to_string()))
                        .ok_or_else(|| {
                            Error::MissingIdentity(Slot::Field(def, scope, name.to_string()))
                        })?;
                    out.required
                        .insert(Requirement::ReadField(target, FieldId(*id)));
                }
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Normalization(crate::norm::Error),
    MissingIdentity(crate::ids::Slot),
    InvalidKernel(&'static str),
    /// The term asks more of the analysis than any one root may.
    TooLarge,
}

/// Returns the requirements of a checked registered query.
pub fn query(schema: &Schema, program: &Program, query: &Query) -> Result<Requirements, Error> {
    let normal = crate::norm::query(schema, program, query).map_err(Error::Normalization)?;
    let program = normal_program(schema, program)?;
    analyze(schema, &program, &normal.terms, normal.body, &normal.params)
}

/// Returns the requirements of a checked registered transaction.
pub fn txn(schema: &Schema, program: &Program, txn: &Txn) -> Result<Requirements, Error> {
    let normal = crate::norm::txn(schema, program, txn).map_err(Error::Normalization)?;
    let program = normal_program(schema, program)?;
    analyze(schema, &program, &normal.terms, normal.body, &normal.params)
}

/// Returns the requirements of `root`, a normal term of `terms` over `params`, with the
/// functions of `program`, the normal program of `schema`.
pub fn normal(
    schema: &Schema,
    program: &NormalProgram,
    terms: &Terms,
    root: Tm,
    params: &[Var],
) -> Result<Requirements, Error> {
    analyze(schema, program, terms, root, params)
}

/// Returns the requirements of each statement in checked ad hoc text.
pub fn adhoc(
    schema: &Schema,
    program: &Program,
    adhoc: &Adhoc,
) -> Result<Vec<Requirements>, Error> {
    let schema = adhoc.schema(schema);
    let normal = crate::norm::adhoc(schema, program, adhoc).map_err(Error::Normalization)?;
    let params: Vec<Var> = normal.params.values().copied().collect();
    let program = normal_program(schema, program)?;
    normal
        .items
        .iter()
        .map(|&root| analyze(schema, &program, &normal.terms, root, &params))
        .collect()
}

fn normal_program(schema: &Schema, program: &Program) -> Result<NormalProgram, Error> {
    crate::norm::program(schema, program).map_err(Error::Normalization)
}

fn analyze(
    schema: &Schema,
    program: &NormalProgram,
    terms: &Terms,
    root: Tm,
    params: &[Var],
) -> Result<Requirements, Error> {
    let mut analyzer = Analyzer {
        schema,
        arenas: [terms, &program.terms],
        aggregates: &program.aggregates,
        requirements: Requirements::default(),
        next_id: 0,
        snapshot: 0,
        steps: 0,
        depth: 0,
    };
    let mut env = Env::new();
    for &var in params {
        env.insert(var, analyzer.unknown(&terms.var_info(var).ty));
    }
    let value = analyzer.eval(Arena::Main, root, &env)?;
    analyzer.consume(&value);
    Ok(analyzer.requirements)
}

/// The most terms the analysis of one root evaluates.
const MAX_STEPS: u32 = 1 << 20;

/// The most nested evaluations of one root, within any thread's default stack.
const MAX_EVAL_DEPTH: u32 = 64;

#[derive(Clone, Copy, Debug)]
enum Arena {
    Main,
    Program,
}

struct Analyzer<'a> {
    schema: &'a Schema,
    arenas: [&'a Terms; 2],
    aggregates: &'a [Option<crate::norm::NormalAggregate>],
    requirements: Requirements,
    next_id: u64,
    snapshot: u64,
    steps: u32,
    depth: u32,
}

impl Analyzer<'_> {
    fn terms(&self, arena: Arena) -> &Terms {
        self.arenas[match arena {
            Arena::Main => 0,
            Arena::Program => 1,
        }]
    }

    fn fresh(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn consume(&mut self, value: &Value) {
        let mut reads = Vec::new();
        value.each_read(&mut |r| reads.push(r.clone()));
        self.charge(reads);
    }

    fn charge(&mut self, reads: Vec<Requirement>) {
        for read in reads {
            if let Requirement::ReadField(target, _) = read {
                self.range(target);
            }
            self.requirements.required.insert(read);
        }
    }

    fn bind_eval(
        &mut self,
        arena: Arena,
        var: Var,
        value: Value,
        body: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let mut inner = env.clone();
        inner.insert(var, value);
        self.eval(arena, body, &inner)
    }
}
