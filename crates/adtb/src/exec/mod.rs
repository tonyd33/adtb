mod code;
mod frame;
mod ir;
mod memory;
mod prim;
mod run;
mod sched;
mod sink;
mod store;
mod types;
mod value;
mod wide;

#[cfg(test)]
mod tests;

pub use ir::{Program, compile};
pub use memory::{Arena, Budget, Grant};
pub use sched::{Scheduler, Task};
pub use store::{AtSnapshot, Reads, Store, StoredHeap, StoredIndex};
pub use value::{F32, F64, Val};

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adtb_lang::kernel::Var;

use crate::batch::{self, Batch, BatchBuilder, Datum};
use crate::mvcc::SerializationFailure;
use crate::runtime::Runtime;
use crate::storage::PageError;

/// A fault of the prelude's `Fault`, which `try` catches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fault {
    DivisionByZero,
    ArithmeticOverflow,
    OutOfRange,
    NotFinite,
    NoSuchDate,
    UnknownZone,
    InvalidText,
    InvalidSpan,
    NegativeCount,
    NoSuchRow,
    FunctionFailed,
}

impl Fault {
    pub const ALL: [Fault; 11] = [
        Fault::DivisionByZero,
        Fault::ArithmeticOverflow,
        Fault::OutOfRange,
        Fault::NotFinite,
        Fault::NoSuchDate,
        Fault::UnknownZone,
        Fault::InvalidText,
        Fault::InvalidSpan,
        Fault::NegativeCount,
        Fault::NoSuchRow,
        Fault::FunctionFailed,
    ];

    /// Returns the name of the prelude constructor.
    pub fn name(self) -> &'static str {
        match self {
            Fault::DivisionByZero => "DivisionByZero",
            Fault::ArithmeticOverflow => "ArithmeticOverflow",
            Fault::OutOfRange => "OutOfRange",
            Fault::NotFinite => "NotFinite",
            Fault::NoSuchDate => "NoSuchDate",
            Fault::UnknownZone => "UnknownZone",
            Fault::InvalidText => "InvalidText",
            Fault::InvalidSpan => "InvalidSpan",
            Fault::NegativeCount => "NegativeCount",
            Fault::NoSuchRow => "NoSuchRow",
            Fault::FunctionFailed => "FunctionFailed",
        }
    }
}

/// The limit a statement ran past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exceeded {
    /// Its memory grant of `limit` bytes.
    Memory { limit: usize },
    /// `limit` rounds of a fixpoint.
    Rounds { limit: usize },
}

/// Why an evaluation produced no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Fault(Fault),
    /// The statement is rejected with `StatementLimit`.
    StatementLimit(Exceeded),
    Serialization(SerializationFailure),
    Page(PageError),
    /// A stored structure the plan reads does not decode.
    Corrupt(String),
    /// The plan needs a form or primitive the executor does not run yet.
    Unsupported(String),
    /// The plan reads `caller` and the statement has none.
    NoCaller,
    Cancelled,
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

/// Bounds and sizes of one statement's execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The rounds a fixpoint may take before the statement is rejected for its limit.
    pub fix_rounds: usize,
    /// The rows of a morsel.
    pub morsel_rows: usize,
    /// The pages a heap may have for its pipelines to run in one task with other such
    /// pipelines; a pipeline with no source always may.
    pub pack_pages: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            fix_rounds: 10_000,
            morsel_rows: 1024,
            pack_pages: 1,
        }
    }
}

/// What one execution of a program runs against.
pub struct Call<R: Runtime> {
    pub store: Arc<Store<R>>,
    pub grant: Arc<Grant>,
    /// The scheduler whose workers run the morsels of top-level queries; without one every
    /// pipeline runs on the calling thread.
    pub pool: Option<Arc<Scheduler<R>>>,
    pub options: Options,
    /// Set to stop the execution at its next morsel boundary.
    pub cancel: Arc<AtomicBool>,
}

impl<R: Runtime> std::fmt::Debug for Call<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("store", &self.store)
            .field("grant", &self.grant)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// Returns the value of `program` with its parameters bound by `params`.
pub fn execute<R: Runtime>(
    rt: R,
    program: Arc<Program>,
    call: Call<R>,
    params: &[(Var, Val)],
) -> Result<Val, Error> {
    let cx = Arc::new(run::Ctx {
        rt,
        program,
        store: call.store,
        grant: call.grant,
        options: call.options,
        pool: call.pool,
        cancel: call.cancel,
    });
    let params: Vec<(u32, Val)> = params.iter().map(|(v, x)| (v.0, x.clone())).collect();
    cx.root(&params)
}

/// How the elements of a result fill the columns of its batches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rows {
    /// Each element's fields are its row's columns.
    Fields,
    /// Each element is its row's one column.
    Whole,
}

/// Returns the elements of the set or list `v` as batches of `schema`, at most `per` rows each.
///
/// Panics if an element is not of the schema's types.
pub fn batches(v: &Val, schema: &batch::Schema, rows: Rows, per: usize) -> Vec<Batch> {
    let elems: Vec<&Val> = match v {
        Val::Set(s) => s.iter().collect(),
        Val::List(xs) => xs.iter().collect(),
        other => panic!("a result in batches is a set or a list, found {other:?}"),
    };
    elems
        .chunks(per.max(1))
        .map(|chunk| {
            let mut b = BatchBuilder::new(schema.clone());
            for e in chunk {
                let row: Vec<Datum> = match rows {
                    Rows::Fields => e
                        .fields()
                        .iter()
                        .zip(schema.fields())
                        .map(|(x, t)| datum(t, x))
                        .collect(),
                    Rows::Whole => vec![datum(&schema.fields()[0], e)],
                };
                b.push_datums(&row);
            }
            b.finish()
        })
        .collect()
}

fn datum(t: &batch::Type, v: &Val) -> Datum {
    match (t, v) {
        (batch::Type::Scalar(s), v) => {
            Datum::Scalar(store::to_key(&crate::key::KeyType::Scalar(*s), v))
        }
        (batch::Type::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => {
            Datum::Record(ts.iter().zip(fs.iter()).map(|(t, x)| datum(t, x)).collect())
        }
        (batch::Type::Sum(cs), Val::Ctor(c, fs)) => Datum::Sum {
            ctor: *c,
            fields: cs[*c as usize]
                .fields
                .iter()
                .zip(fs.iter())
                .map(|(t, x)| datum(t, x))
                .collect(),
        },
        (batch::Type::List(e), Val::Set(s)) => Datum::List(s.iter().map(|x| datum(e, x)).collect()),
        (batch::Type::List(e), Val::List(xs)) => {
            Datum::List(xs.iter().map(|x| datum(e, x)).collect())
        }
        (t, v) => panic!("{v:?} is not a value of batch type {t:?}"),
    }
}
