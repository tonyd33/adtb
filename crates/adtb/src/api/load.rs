//! Bulk append: Arrow record batches staged per constructor relation and inserted as one change
//! set at commit.

use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{Base, RelId, Ty};

use crate::catalog::layout::{self, batch_type};
use crate::exec::{self, Conv, Val};
use crate::runtime::Runtime;

use super::arrow::{ArrowSchema, Field, RecordBatch};
use super::error::{Error, RejectReason};
use super::stop::CancelToken;
use super::txn::{AdHocTransaction, TxnCallOptions, TxnOptions};
use super::typed::CtorRelation;
use super::{Database, Durability, Position};

/// The options of a load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadOptions {
    pub durability: Durability,
    pub cancel: Option<CancelToken>,
}

/// What a committed load inserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub rows: u64,
    pub position: Position,
}

/// A bulk append into constructor relations: rows pushed to its appenders are staged, and commit
/// inserts them all as one change set of one write transaction. Dropping it without a commit
/// discards what it staged.
#[derive(Debug)]
pub struct Load<'a, R: Runtime = adtb_sys::os::OsRuntime> {
    txn: AdHocTransaction<'a, R>,
    cancel: Option<CancelToken>,
    staged: Vec<Staged>,
}

/// One field of a constructor relation's row.
#[derive(Debug)]
struct Column {
    name: String,
    ty: Ty,
    conv: Conv,
    /// Whether a key generator or a default fills the field when a batch leaves it out.
    filled: bool,
}

/// The rows staged for one constructor relation, by the fields their batches held.
#[derive(Debug)]
struct Staged {
    /// The constructor relation as ad hoc text names it.
    target: String,
    rel: RelId,
    /// The positions among `columns` of the key fields, in name order.
    key: Vec<usize>,
    columns: Vec<Column>,
    schema: Arc<ArrowSchema>,
    /// The positions among `columns` of each batch's fields, with its rows, each a value per
    /// field.
    rows: Vec<(Vec<usize>, Vec<Vec<Val>>)>,
}

/// Where the batches of one constructor relation of a load go.
#[derive(Debug)]
pub struct Appender<'l> {
    staged: &'l mut Staged,
}

impl<R: Runtime> Database<R> {
    /// Open a load, a write transaction whose snapshot is taken now.
    pub fn load(&self, opts: &LoadOptions) -> Result<Load<'_, R>, Error> {
        let txn = self.begin(&TxnOptions {
            durability: opts.durability,
            ..TxnOptions::default()
        })?;
        Ok(Load {
            txn,
            cancel: opts.cancel.clone(),
            staged: Vec::new(),
        })
    }
}

impl<R: Runtime> Load<'_, R> {
    /// Returns the appender of constructor relation `target`, as ad hoc text names it:
    /// `r.C`, or `r` for a relation of a record type, its module qualifying `r` where needed.
    ///
    /// Fail with `Invalid` when `target` names no constructor relation.
    pub fn appender<C: CtorRelation>(&mut self, target: C) -> Result<Appender<'_>, Error> {
        let fresh = staged(self.txn.engine.schema.as_ref(), target.target())?;
        let at = match self.staged.iter().position(|s| s.target == fresh.target) {
            Some(at) => at,
            None => {
                self.staged.push(fresh);
                self.staged.len() - 1
            }
        };
        Ok(Appender {
            staged: &mut self.staged[at],
        })
    }

    /// Insert every staged row of every appender as one change set and commit. Returns the rows
    /// inserted and the commit position.
    ///
    /// Fail, inserting nothing, as an insert of the rows fails: a key two staged rows share
    /// with `KeyViolation`, a broken constraint with `Violated`, and a collision with a row
    /// committed since the load opened with `Serialization`. The engine does not rerun it.
    pub fn commit(mut self) -> Result<LoadReport, Error> {
        for s in &self.staged {
            if let Some(key) = s.shared_key() {
                return Err(Error::key_violation(&self.txn.engine.schema, s.rel, &key));
            }
        }
        let mut text = String::from("transact\n");
        let mut params = Vec::new();
        let mut rows = 0;
        for s in &self.staged {
            for (fields, staged) in &s.rows {
                let name = format!("load_rows_{}", params.len());
                let picked = fields
                    .iter()
                    .map(|&f| format!("{0}: x.{0}", s.columns[f].name))
                    .collect::<Vec<_>>()
                    .join(", ");
                text.push_str(&format!(
                    "  insert {} from (from x in {name} select {{ {picked} }})\n",
                    s.target
                ));
                rows += staged.len() as u64;
                let (ty, value) = s.param(fields, staged);
                params.push((name, ty, value));
            }
        }
        if params.is_empty() {
            let position = self.txn.commit(None)?;
            return Ok(LoadReport { rows, position });
        }
        text.push_str("  return {}\n");
        let params: Vec<(&str, Ty, Val)> = params
            .iter()
            .map(|(n, t, v)| (n.as_str(), t.clone(), v.clone()))
            .collect();
        let opts = TxnCallOptions {
            cancel: self.cancel.clone(),
            ..TxnCallOptions::default()
        };
        self.txn.query_text(&text, &params, &opts)?;
        let position = self.txn.commit(None)?;
        Ok(LoadReport { rows, position })
    }
}

impl Appender<'_> {
    /// Returns the schema of a batch holding every field of the constructor relation's row.
    pub fn schema(&self) -> &Arc<ArrowSchema> {
        &self.staged.schema
    }

    /// Stage the rows of `batch`, unconstrained until the load commits.
    ///
    /// Fail with `SchemaMismatch`, staging nothing, unless the batch holds the fields of
    /// [`schema`](Appender::schema), in order and at their types, less any a key generator or
    /// a default fills.
    pub fn push(&mut self, batch: &RecordBatch) -> Result<(), Error> {
        let s = &mut *self.staged;
        let fields = matched(&s.columns, &s.schema.fields, &batch.schema().fields)
            .ok_or_else(|| mismatch(&s.target, &batch.schema().fields))?;
        let b = batch.batch();
        let rows: Vec<Vec<Val>> = (0..b.len())
            .map(|i| {
                fields
                    .iter()
                    .enumerate()
                    .map(|(j, &f)| {
                        let conv = &s.columns[f].conv;
                        exec::canonical(exec::from_stored(conv, &b.value(j, i)))
                    })
                    .collect()
            })
            .collect();
        match s.rows.iter_mut().find(|(f, _)| *f == fields) {
            Some((_, staged)) => staged.extend(rows),
            None => s.rows.push((fields, rows)),
        }
        Ok(())
    }
}

impl Staged {
    /// Returns the least key that two staged rows give.
    fn shared_key(&self) -> Option<Val> {
        let mut seen = BTreeSet::new();
        let mut shared = BTreeSet::new();
        for (fields, rows) in &self.rows {
            let Some(at) = self
                .key
                .iter()
                .map(|k| fields.iter().position(|f| f == k))
                .collect::<Option<Vec<usize>>>()
            else {
                continue;
            };
            for row in rows {
                let key = match at.as_slice() {
                    [j] => row[*j].clone(),
                    _ => Val::record(at.iter().map(|&j| row[j].clone()).collect()),
                };
                if !seen.insert(key.clone()) {
                    shared.insert(key);
                }
            }
        }
        shared.into_iter().next()
    }

    /// Returns the type and value of a parameter holding `rows`, each a value per field of
    /// `fields`: a set of records of those fields and an ordinal, which keeps equal rows apart.
    fn param(&self, fields: &[usize], rows: &[Vec<Val>]) -> (Ty, Val) {
        let mut ordinal = String::from("load_ordinal");
        while self.columns.iter().any(|c| c.name == ordinal) {
            ordinal.push('_');
        }
        let mut named: Vec<(String, Ty, Option<usize>)> = fields
            .iter()
            .enumerate()
            .map(|(j, &f)| {
                (
                    self.columns[f].name.clone(),
                    self.columns[f].ty.clone(),
                    Some(j),
                )
            })
            .collect();
        named.push((ordinal, Ty::Base(Base::I64), None));
        named.sort_by(|a, b| a.0.cmp(&b.0));
        let records: BTreeSet<Val> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                Val::record(
                    named
                        .iter()
                        .map(|(_, _, j)| match j {
                            Some(j) => row[*j].clone(),
                            None => Val::Int(i64::try_from(i).unwrap_or(i64::MAX)),
                        })
                        .collect(),
                )
            })
            .collect();
        let ty = Ty::Record(named.into_iter().map(|(n, t, _)| (n, t)).collect());
        (Ty::Set(Box::new(ty)), Val::set(records))
    }
}

/// Returns the position among `columns` of each field of `given`, or `None` unless `given` is
/// `full` less some fields that are filled.
fn matched(columns: &[Column], full: &[Field], given: &[Field]) -> Option<Vec<usize>> {
    let mut out = Vec::with_capacity(given.len());
    let mut next = given.iter().peekable();
    for (i, (c, f)) in columns.iter().zip(full).enumerate() {
        if next.next_if(|g| *g == f).is_some() {
            out.push(i);
        } else if !c.filled {
            return None;
        }
    }
    next.peek().is_none().then_some(out)
}

fn mismatch(target: &str, given: &[Field]) -> Error {
    let names: Vec<&str> = given.iter().map(|f| f.name.as_str()).collect();
    Error::rejected(RejectReason::SchemaMismatch {
        detail: format!(
            "a batch of fields ({}) is not of the row type of {target}",
            names.join(", ")
        ),
    })
}

/// Returns the empty staging of `target`, a constructor relation of `schema` as ad hoc text
/// names it.
fn staged(schema: &Schema, target: &str) -> Result<Staged, Error> {
    let unknown = || Error::invalid(format!("`{target}` names no constructor relation"));
    let (rel_name, ctor_name) = match target.rsplit_once('.') {
        Some((r, c)) => (r, Some(c)),
        None => (target, None),
    };
    let rel = relation(schema, rel_name).ok_or_else(unknown)?;
    let def = schema.ty(schema.relation(rel).elem_def);
    let ctor = match ctor_name {
        Some(c) => def.ctor_index(c).ok_or_else(unknown)?,
        None if def.ctors().len() == 1 => 0,
        None => return Err(unknown()),
    };
    let r = schema.relation(rel);
    let generated: BTreeSet<&str> = match (&r.generate, r.key.as_slice()) {
        (Some(_), [k]) => BTreeSet::from([k.as_str()]),
        _ => BTreeSet::new(),
    };
    let mut columns = Vec::new();
    let mut fields = Vec::new();
    for (name, ty) in layout::row(schema, rel, ctor) {
        let filled = generated.contains(name.as_str())
            || r.defaults
                .iter()
                .any(|d| d.field == name && d.ctor.is_none_or(|c| c == ctor));
        let column = batch_type(schema, &ty).ok_or_else(unknown)?;
        fields.push(Field {
            name: name.clone(),
            ty: column,
        });
        columns.push(Column {
            conv: exec::conv(schema, &ty),
            name,
            ty,
            filled,
        });
    }
    let mut key_names: Vec<&String> = r.key.iter().collect();
    key_names.sort();
    let key = key_names
        .iter()
        .map(|k| columns.iter().position(|c| c.name == **k))
        .collect::<Option<Vec<usize>>>()
        .ok_or_else(unknown)?;
    let qualified = format!("{}::{}", schema.module(r.module).name, r.name);
    Ok(Staged {
        rel,
        key,
        target: match ctor_name {
            Some(c) => format!("{qualified}.{c}"),
            None => qualified,
        },
        columns,
        schema: Arc::new(ArrowSchema { fields }),
        rows: Vec::new(),
    })
}

/// Returns the relation `name`: `m::r`, or `r` when one module of `schema` the engine does not
/// declare declares it.
fn relation(schema: &Schema, name: &str) -> Option<RelId> {
    let (module, rel) = match name.rsplit_once("::") {
        Some((m, r)) => (Some(m), r),
        None => (None, name),
    };
    let found: Vec<RelId> = schema
        .modules
        .iter()
        .filter(|m| module.map_or(!m.engine, |want| m.name == want))
        .filter_map(|m| match m.values.get(rel) {
            Some(ValueDef::Relation(r)) => Some(*r),
            _ => None,
        })
        .collect();
    match found.as_slice() {
        [r] => Some(*r),
        _ => None,
    }
}
