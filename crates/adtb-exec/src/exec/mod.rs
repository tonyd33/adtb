//! Compile physical plans and run them on morsels of batches.

mod broken;
mod class;
mod code;
mod frame;
mod host;
mod ir;
mod join;
pub mod layout;
mod memory;
mod passed;
mod prim;
mod reads;
mod rot;
mod run;
mod sched;
mod sink;
mod spill;
mod store;
mod stream;
pub mod types;
mod value;
mod wide;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod textual_tests;

pub use crate::mvcc::StopsEarly;
pub use broken::Broken;
pub use class::Class;
pub use host::admit;
pub use host::{HostBody, HostError, Hosts};
pub use ir::{Bound, Program, compile};
pub use memory::{Arena, Budget, Footprint, Grant, Held, Released, Requested, Shared, Ticket};
pub use prim::print;
#[cfg(any(test, feature = "testing"))]
pub use reads::Reads;
pub use rot::{Rot, detail};
#[cfg(any(test, feature = "testing"))]
pub use sched::{Admission, Task};
pub use sched::{Admitted, Entry, Priority, Refusal, Refused, Resumable, Scheduler, Step, Stopped};
pub use spill::{Distinct, Merge, Spill};
pub use store::{
    Bindings, Histories, IndexedRows, Listing, Named, PastReads, ProbeBounds, Routes, RowKeys,
    Store, StoredHeap, StoredIndex,
};
pub use store::{Conv, conv, from_stored, from_stored_row, to_key, to_stored};
pub use stream::{Produced, Production, produce};
#[cfg(any(test, feature = "testing"))]
pub use value::F32;
pub use value::{F64, Val};

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use adtb_lang::kernel::{Tm, Valid, Var};
use adtb_lang::schema::Schema;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::textual::PrintError;

use crate::batch::{self, Batch, BatchBuilder, Datum};
use crate::mvcc::Conflict;
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

    /// Returns the position of the prelude constructor.
    pub const fn ctor(self) -> u32 {
        match self {
            Fault::DivisionByZero => ctor::DIVISION_BY_ZERO,
            Fault::ArithmeticOverflow => ctor::ARITHMETIC_OVERFLOW,
            Fault::OutOfRange => ctor::OUT_OF_RANGE,
            Fault::NotFinite => ctor::NOT_FINITE,
            Fault::NoSuchDate => ctor::NO_SUCH_DATE,
            Fault::UnknownZone => ctor::UNKNOWN_ZONE,
            Fault::InvalidText => ctor::INVALID_TEXT,
            Fault::InvalidSpan => ctor::INVALID_SPAN,
            Fault::NegativeCount => ctor::NEGATIVE_COUNT,
            Fault::NoSuchRow => ctor::NO_SUCH_ROW,
            Fault::FunctionFailed => ctor::FUNCTION_FAILED,
        }
    }

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
    Serialization(Conflict),
    Page(PageError),
    /// An entry of the index with this owner names an unused slot of a page its heap holds.
    OrphanEntry(u32),
    /// The call's spill file could not be made, written or read.
    Spill(IoError),
    /// A stored structure or spill file the plan reads does not decode.
    Corrupt(Rot),
    /// The plan needs a form or primitive the executor does not run yet.
    Unsupported(String),
    /// The plan reads `caller` and the statement has none.
    NoCaller,
    /// The evaluation reached `reject` with this text.
    Rejected(String),
    Cancelled,
    /// The queue of calls waiting for memory was full.
    Overloaded,
    /// The call's deadline passed.
    DeadlineExceeded,
    /// The evaluation reached a host function, by its name, with no body bound.
    MissingFunction(String),
    /// The evaluation reached a host function whose body is bound at another version than its
    /// declaration names; the name the body is registered under.
    OtherVersion(String),
    /// The plan or a value breaks an invariant the checker guarantees.
    Internal(Box<Broken>),
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

impl From<batch::TooLarge> for Error {
    fn from(_: batch::TooLarge) -> Error {
        Broken::PastOffsets.into()
    }
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<crate::heap::OrphanEntry> for Error {
    fn from(e: crate::heap::OrphanEntry) -> Error {
        Error::OrphanEntry(e.index)
    }
}

impl From<PrintError> for Error {
    fn from(e: PrintError) -> Error {
        Broken::Print(e).into()
    }
}

/// Returns whether the compiled scalar term `t` of `terms` routes rows among several arms by
/// their constructor.
#[cfg(test)]
pub(crate) fn routes_by_tag(
    schema: &Schema,
    terms: &adtb_lang::kernel::Terms,
    t: Tm,
) -> Result<bool, Error> {
    Ok(code::Compiler::new(schema, terms).code(t)?.routes_by_tag())
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
    /// Compile `body`, a scalar normal form in `terms` for `schema` whose one free variable is
    /// `var`.
    pub fn new(schema: &Schema, terms: Valid<'_>, var: Var, body: Tm) -> Result<Scalar, Error> {
        let terms = terms.terms();
        Ok(Scalar {
            code: code::Compiler::new(schema, terms).code(body)?,
            var,
            vars: terms.vars().len(),
            shared: code::Shared {
                caller: None,
                grant: None,
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

/// Returns the arm of `arms` for a scrutinee of constructor `c`. Fail with
/// `Broken::Unreachable` for a constructor the scrutinee's type excludes, whose arm is `None`.
fn arm<T>(arms: &[Option<T>], c: u32) -> Result<&T, Error> {
    arms.get(c as usize)
        .and_then(Option::as_ref)
        .ok_or_else(|| Broken::Unreachable.into())
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

impl Options {
    /// Returns these options bounding a fixpoint at `rounds`, or at these options' bound with
    /// `None`.
    pub fn with_fix_rounds(self, rounds: Option<usize>) -> Options {
        Options {
            fix_rounds: rounds.unwrap_or(self.fix_rounds),
            ..self
        }
    }
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
    store: Arc<Store<R>>,
    grant: Arc<Grant>,
    /// The scheduler whose workers run the morsels of top-level queries; without one every
    /// pipeline runs on the calling thread.
    pool: Option<Arc<Scheduler<R>>>,
    options: Options,
    /// Set to stop the execution at its next morsel boundary.
    cancel: Arc<AtomicBool>,
    deadline: Option<Deadline>,
}

impl<R: Runtime> Call<R> {
    /// Returns the call that runs over `store` within `grant` under `options`, every pipeline on
    /// the calling thread, stopped at its next morsel boundary once `cancel` is set or
    /// `deadline` passes.
    pub fn new(
        store: Arc<Store<R>>,
        grant: Arc<Grant>,
        options: Options,
        cancel: Arc<AtomicBool>,
        deadline: Option<Deadline>,
    ) -> Call<R> {
        Call {
            store,
            grant,
            pool: None,
            options,
            cancel,
            deadline,
        }
    }

    /// Returns this call running the morsels of top-level queries on the workers of `pool`.
    pub fn on(self, pool: Option<Arc<Scheduler<R>>>) -> Call<R> {
        Call { pool, ..self }
    }
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

/// Returns the value of the bound program `bound`, held in the call's grant.
pub fn execute<R: Runtime>(rt: R, bound: Bound, call: Call<R>) -> Result<Held<Val>, Error> {
    let grant = call.grant.clone();
    let (cx, env) = start(rt, bound, call)?;
    grant.hold(cx.root(&env)?)
}

/// Returns the context that runs the program of `bound` for `call` over its store narrowed to the
/// program's reads, and the frame binding its parameters.
fn start<R: Runtime>(
    rt: R,
    bound: Bound,
    call: Call<R>,
) -> Result<(Arc<run::Ctx<R>>, frame::Frame), Error> {
    let (program, params) = bound.into_parts();
    let store = Arc::new(call.store.narrowed(program.reads())?);
    let mut env = frame::Frame::unit(program.vars);
    for (v, x) in params {
        env.bind_one(v.0, x);
    }
    let shared = code::Shared {
        grant: Some(call.grant.clone()),
        ..program.shared.clone()
    };
    let cx = Arc::new(run::Ctx {
        spill: Spill::new(rt.clone(), call.grant.clone(), call.options.spill_bytes),
        rt,
        shared,
        program,
        store,
        grant: call.grant,
        options: call.options,
        pool: call.pool,
        cancel: call.cancel,
        deadline: call.deadline,
    });
    Ok((cx, env))
}

/// How the elements of a result fill the columns of its batches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rows {
    /// Each element's fields are its row's columns.
    Fields,
    /// Each element is its row's one column.
    Whole,
}

/// Returns the elements of the set or list `v` as batches of `schema`, at most `per` rows each,
/// a batch cut sooner where the next element would take a column of it past 2^31 elements or
/// value bytes.
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
            return Err(Broken::NotBatchable(other.clone()).into());
        }
    };
    let mut out = Vec::new();
    let mut b = BatchBuilder::new(schema.clone());
    for e in elems {
        let row = match rows {
            Rows::Fields => data(schema.fields(), e.fields()?)?,
            Rows::Whole => vec![datum(&schema.fields()[0], e)?],
        };
        out.extend(b.push_datums_or_cut(&row)?);
        if b.len() == per.max(1) {
            out.push(std::mem::replace(&mut b, BatchBuilder::new(schema.clone())).finish());
        }
    }
    if !b.is_empty() {
        out.push(b.finish());
    }
    Ok(out)
}

fn mistyped(v: &Val, t: &batch::Type) -> Error {
    Broken::NotBatchTyped {
        value: v.clone(),
        ty: t.clone(),
    }
    .into()
}

fn data(ts: &[batch::Type], vs: &[Val]) -> Result<Vec<Datum>, Error> {
    if ts.len() != vs.len() {
        return Err(Broken::FieldCount {
            values: vs.to_vec(),
            types: ts.to_vec(),
        }
        .into());
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
