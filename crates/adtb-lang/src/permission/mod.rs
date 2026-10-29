use std::collections::{BTreeMap, BTreeSet};

use crate::check::{Adhoc, Program, Query, Txn};
use crate::kernel::{Terms, Tm, Var};
use crate::norm::NormalProgram;
use crate::schema::Schema;
use crate::types::*;

mod aggregate;
mod coverage;
mod eval;
mod shape;
mod value;
mod write;

#[cfg(test)]
mod tests;

pub use coverage::{EffectiveGrant, EffectiveGrants, Mode, Right};
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

impl Requirements {
    pub fn iter(&self) -> impl Iterator<Item = &Requirement> {
        self.required.iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Normalization(crate::norm::Error),
    MissingIdentity(crate::ids::Slot),
    InvalidKernel(&'static str),
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
    };
    let mut env = Env::new();
    for &var in params {
        env.insert(var, analyzer.unknown(&terms.var_info(var).ty));
    }
    let value = analyzer.eval(Arena::Main, root, &env)?;
    analyzer.consume(&value);
    Ok(analyzer.requirements)
}

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
