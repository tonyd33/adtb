mod code;
mod frame;
mod host;
mod ir;
mod join;
mod memory;
mod prim;
mod run;
mod sched;
mod sink;
mod spill;
mod store;
mod stream;
pub(crate) mod types;
mod value;
mod wide;

#[cfg(test)]
mod tests;

pub(crate) use host::canonical;
pub use host::{HostBody, HostCall, HostError, Hosts};
pub use ir::{Program, compile};
pub use memory::{Arena, Budget, Grant};
pub(crate) use prim::print;
pub use sched::{
    Admission, Admitted, Entry, Priority, Refusal, Refused, Resumable, Scheduler, Step, Stopped,
    Task,
};
pub use store::{
    AtSnapshot, Histories, IndexedRows, Listing, PastReads, ProbeBounds, Reads, Store, StoredHeap,
    StoredIndex,
};
pub(crate) use store::{Conv, conv, from_stored, to_key, to_stored};
pub use stream::{Produced, Production, produce};
pub use value::{F32, F64, Val};
pub(crate) use wide::decimal_fits;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::schema::Schema;
use adtb_lang::textual::PrintError;

use crate::batch::{self, Batch, BatchBuilder, Datum};
use crate::mvcc::SerializationFailure;
use crate::runtime::{Instant, IoError, Runtime};
use crate::storage::{PageError, Position};

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
    /// Its spill bound of `limit` bytes, spilling what its memory grant of `grant` bytes could
    /// not hold.
    Spill { limit: usize, grant: usize },
    /// `limit` rounds of a fixpoint.
    Rounds { limit: usize },
    /// Its time limit, `after` from its start.
    Time { after: Duration },
}

/// Why an evaluation produced no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Fault(Fault),
    /// The statement is rejected with `StatementLimit`.
    StatementLimit(Exceeded),
    Serialization(SerializationFailure),
    Page(PageError),
    /// The call's spill file could not be made, written or read.
    Spill(IoError),
    /// A stored structure the plan reads does not decode.
    Corrupt(String),
    /// The plan needs a form or primitive the executor does not run yet.
    Unsupported(String),
    /// The plan reads `caller` and the statement has none.
    NoCaller,
    /// The evaluation reached `reject` with this text.
    Rejected(String),
    Cancelled,
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The evaluation reached a host function, by its name, with no body bound.
    MissingFunction(String),
    /// The evaluation reached a host function whose body is bound at another version than its
    /// declaration names; the name the body is registered under.
    OtherVersion(String),
    /// The plan or a value breaks the invariant this text names, one the checker guarantees.
    Internal(String),
    /// A read of the past names a state the engine keeps no history of; `oldest` is the oldest
    /// position a read may name.
    HistoryUnavailable {
        oldest: Position,
    },
    /// A read of the past names a state at schema version `version`, which defines what it reads
    /// otherwise than the version it is typed at.
    WrongVersion {
        version: u64,
    },
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<PrintError> for Error {
    fn from(e: PrintError) -> Error {
        Error::Internal(e.message)
    }
}

/// A scalar expression of one variable, compiled.
#[derive(Clone, Debug)]
pub struct Scalar {
    code: Arc<code::Code>,
    var: Var,
    vars: usize,
    shared: code::Shared,
}

impl Scalar {
    /// Compiles `body`, a scalar normal form in `terms` for `schema` whose one free variable is
    /// `var`.
    pub fn new(schema: &Schema, terms: &Terms, var: Var, body: Tm) -> Result<Scalar, Error> {
        Ok(Scalar {
            code: code::Compiler::new(schema, terms).code(body)?,
            var,
            vars: terms.vars.len(),
            shared: code::Shared {
                caller: None,
                fault_ctors: ir::fault_ctors(schema),
                hosts: Arc::default(),
            },
        })
    }

    /// Bind the host function bodies of `hosts` to the calls the expression makes.
    pub fn set_hosts(&mut self, hosts: Arc<Hosts>) {
        self.shared.hosts = hosts;
    }

    /// Returns the expression's value with its variable bound to `v`.
    pub fn eval(&self, v: Val) -> Result<Val, Error> {
        let mut f = frame::Frame::unit(self.vars);
        f.bind_one(self.var.0, v);
        self.code.eval_one(&mut f, &self.shared)
    }
}

/// Returns the arm of `arms` for a scrutinee of constructor `c`.
fn arm<T>(arms: &[Option<T>], c: u32) -> Result<&T, Error> {
    arms.get(c as usize)
        .and_then(Option::as_ref)
        .ok_or_else(|| Error::Internal(format!("a case has an arm for constructor {c}")))
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
    /// The pages of a heap one task of a parallel scan reads.
    pub range_pages: usize,
    /// The bytes a statement may write to its spill file.
    pub spill_bytes: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            fix_rounds: 10_000,
            morsel_rows: 1024,
            pack_pages: 1,
            range_pages: 8,
            spill_bytes: usize::MAX,
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
    pub deadline: Option<Deadline>,
}

/// An instant of the runtime's clock at which an execution stops at its next morsel boundary,
/// failing with `error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deadline {
    pub at: Instant,
    pub error: Error,
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
    let (cx, env) = start(rt, program, call, params);
    cx.root(&env)
}

/// Returns the context that runs `program` for `call`, and the frame binding its parameters by
/// `params`.
fn start<R: Runtime>(
    rt: R,
    program: Arc<Program>,
    call: Call<R>,
    params: &[(Var, Val)],
) -> (Arc<run::Ctx<R>>, frame::Frame) {
    let mut env = frame::Frame::unit(program.vars);
    for (v, x) in params {
        env.bind_one(v.0, x.clone());
    }
    let cx = Arc::new(run::Ctx {
        spill: spill::Spill::new(rt.clone(), call.grant.clone(), call.options.spill_bytes),
        rt,
        program,
        store: call.store,
        grant: call.grant,
        options: call.options,
        pool: call.pool,
        cancel: call.cancel,
        deadline: call.deadline,
    });
    (cx, env)
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
/// Returns `Error::Internal` if `v` is not a set or list or an element is not of the schema's
/// types.
pub fn batches(
    v: &Val,
    schema: &batch::Schema,
    rows: Rows,
    per: usize,
) -> Result<Vec<Batch>, Error> {
    let elems: Vec<&Val> = match v {
        Val::Set(s) => s.iter().collect(),
        Val::List(xs) => xs.iter().collect(),
        other => {
            return Err(Error::Internal(format!(
                "a result in batches is a set or a list, found {other:?}"
            )));
        }
    };
    elems
        .chunks(per.max(1))
        .map(|chunk| {
            let mut b = BatchBuilder::new(schema.clone());
            for e in chunk {
                let row = match rows {
                    Rows::Fields => data(schema.fields(), e.fields()?)?,
                    Rows::Whole => vec![datum(&schema.fields()[0], e)?],
                };
                b.push_datums(&row);
            }
            Ok(b.finish())
        })
        .collect()
}

fn mistyped(v: &Val, t: &batch::Type) -> Error {
    value::mistyped(&format!("a value of batch type {t:?}"), v)
}

fn data(ts: &[batch::Type], vs: &[Val]) -> Result<Vec<Datum>, Error> {
    if ts.len() != vs.len() {
        return Err(Error::Internal(format!(
            "{vs:?} has {} fields, its batch type {ts:?} has {}",
            vs.len(),
            ts.len()
        )));
    }
    ts.iter().zip(vs).map(|(t, x)| datum(t, x)).collect()
}

pub(super) fn datum(t: &batch::Type, v: &Val) -> Result<Datum, Error> {
    Ok(match (t, v) {
        (batch::Type::Scalar(s), v) => {
            Datum::Scalar(to_key(&crate::key::KeyType::Scalar(*s), v).map_err(|_| mistyped(v, t))?)
        }
        (batch::Type::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => {
            Datum::Record(data(ts, fs)?)
        }
        (batch::Type::Sum(cs), Val::Ctor(c, fs)) => Datum::Sum {
            ctor: *c,
            fields: data(
                &cs.get(*c as usize).ok_or_else(|| mistyped(v, t))?.fields,
                fs,
            )?,
        },
        (batch::Type::List(e), Val::Set(s)) => {
            Datum::List(s.iter().map(|x| datum(e, x)).collect::<Result<_, _>>()?)
        }
        (batch::Type::List(e), Val::List(xs)) => {
            Datum::List(xs.iter().map(|x| datum(e, x)).collect::<Result<_, _>>()?)
        }
        (t, v) => return Err(mistyped(v, t)),
    })
}
