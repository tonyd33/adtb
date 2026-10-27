//! Host functions: bodies the embedding program supplies, each called once per batch of rows.

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crate::batch::{self, Batch, BatchBuilder};

use super::store::{Conv, from_stored};
use super::{Error, Fault, Val, datum};

/// Why a host function's body failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostError(pub String);

/// A host function's body. It takes a batch of one column per parameter and returns a batch of
/// one column of the result type, with as many rows.
pub type HostBody = dyn Fn(&Batch) -> Result<Batch, HostError> + Send + Sync;

/// The bodies of the host functions bound to an open database, by the name their declarations
/// register them under.
#[derive(Clone, Default)]
pub struct Hosts(BTreeMap<String, Arc<HostBody>>);

impl fmt::Debug for Hosts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl Hosts {
    pub fn insert(&mut self, name: impl Into<String>, body: Arc<HostBody>) {
        self.0.insert(name.into(), body);
    }
}

/// A call of one host function, as code runs it.
#[derive(Clone, Debug)]
pub struct HostCall {
    /// The function's name in the schema.
    pub function: String,
    /// The name its body is registered under.
    pub symbol: String,
    pub params: Vec<batch::Type>,
    pub result: batch::Type,
    pub conv: Conv,
}

impl HostCall {
    /// Call the body bound for this function once with `rows`, the arguments of each row.
    /// Returns the result of each row, or the fault of every row when the body fails, panics, or
    /// returns a batch of another type or length.
    ///
    /// Fails with `MissingFunction` when no body is bound.
    pub fn call(
        &self,
        hosts: &Hosts,
        rows: &[Vec<&Val>],
    ) -> Result<Result<Vec<Val>, Fault>, Error> {
        let body = hosts
            .0
            .get(&self.symbol)
            .ok_or_else(|| Error::MissingFunction(self.function.clone()))?;
        let mut b = BatchBuilder::new(batch::Schema::new(self.params.clone()));
        for row in rows {
            let ds = self
                .params
                .iter()
                .zip(row)
                .map(|(t, v)| datum(t, v))
                .collect::<Result<Vec<_>, _>>()?;
            b.push_datums(&ds);
        }
        let args = b.finish();
        let ran = catch_unwind(AssertUnwindSafe(|| {
            let out = body(&args).ok()?;
            let fits = out.len() == rows.len() && out.schema().fields() == [self.result.clone()];
            fits.then(|| {
                (0..out.len())
                    .map(|i| canonical(from_stored(&self.conv, &out.value(0, i))))
                    .collect()
            })
        }));
        Ok(ran.ok().flatten().ok_or(Fault::FunctionFailed))
    }
}

/// Returns `v` with every text normalized to NFC, as construction normalizes it.
pub(crate) fn canonical(v: Val) -> Val {
    match v {
        Val::Text(t) => Val::text(&super::prim::unicode::nfc(&t)),
        Val::Ctor(c, fs) => Val::ctor(c, fs.iter().cloned().map(canonical).collect()),
        Val::Record(fs) => Val::record(fs.iter().cloned().map(canonical).collect()),
        other => other,
    }
}
