//! Typed bundles: the host types and callables code generation emits for a bundle file, called
//! through its registration.

use std::fmt;
use std::marker::PhantomData;

pub use adtb_lang::textual::{Scalar, Textual};
pub use adtb_lang::types::Base;
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::runtime::Runtime;

use super::arrow::ArrowStream;
use super::error::{Error, FatalReason, RejectReason};
use super::registered::{Called, InTxn, Registered, Registration};
use super::text::DynRows;
use super::txn::{AdHocTransaction, Committed, TxnCallOptions, TxnOptions};
use super::{CallOptions, Database, Durability, Position};

/// A host type of a language type.
pub trait HostValue: Sized {
    fn to_value(&self) -> Val;
    /// Returns `None` at a value of another type.
    fn from_value(v: &Val) -> Option<Self>;
}

/// A bundle file code generation emitted.
pub trait BundleDef {
    /// The bundle's error type, for the edition the bundle names.
    type Error: From<Error> + fmt::Debug;
    /// The bundle's name, `family-version`.
    const NAME: &'static str;
    /// The bundle file, header included.
    const TEXT: &'static str;
    /// The version the header pins.
    const PINS: u64;

    /// Returns the registration open takes for the bundle.
    ///
    /// Fail with `Invalid` when the header states a clause the engine does not run.
    fn registration(&self) -> Result<Registration, Error> {
        Registration::bundle(Self::TEXT)
    }
}

/// A query, transaction or imported function of a bundle.
pub trait Callable {
    type Bundle: BundleDef;
    /// The arguments, a tuple of one host value per declared parameter.
    type Args: Args;
    /// What a call outside an interactive transaction returns.
    type Output;
    /// What a call inside an interactive transaction returns.
    type InTxn;
    const ORDINAL: u32;

    /// Returns the output of a call that returned `c`.
    fn output(c: Called) -> Result<Self::Output, Error>;
    /// Returns the result of a call inside a transaction that returned `c`.
    fn in_txn(c: InTxn) -> Result<Self::InTxn, Error>;
}

/// The arguments of a call.
pub trait Args {
    /// Returns one value per declared parameter, in order.
    fn values(&self) -> Vec<Val>;
}

macro_rules! args {
    ($($t:ident $i:tt),*) => {
        impl<$($t: HostValue),*> Args for ($($t,)*) {
            fn values(&self) -> Vec<Val> {
                vec![$(self.$i.to_value()),*]
            }
        }
    };
}

args!();
args!(A 0);
args!(A 0, B 1);
args!(A 0, B 1, C 2);
args!(A 0, B 1, C 2, D 3);
args!(A 0, B 1, C 2, D 3, E 4);
args!(A 0, B 1, C 2, D 3, E 4, F 5);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
args!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);

/// A relation variable code generation emitted, typing its keys.
pub trait Relation {
    /// The host type of the relation's key value: its key field, or a record of its key fields.
    type KeyValue: HostValue;
    /// The relation's name as ad hoc text names it.
    const NAME: &'static str;
    /// The relation variable's catalog id.
    const ID: u64;
}

/// A constructor relation a load fills: `r.C`, or `r` of a record type.
pub trait CtorRelation {
    /// Returns its name as ad hoc text names it.
    fn target(&self) -> &str;
}

impl CtorRelation for &str {
    fn target(&self) -> &str {
        self
    }
}

/// A key of relation `R`.
pub struct Key<R: Relation>(pub R::KeyValue);

impl<R: Relation> Clone for Key<R>
where
    R::KeyValue: Clone,
{
    fn clone(&self) -> Self {
        Key(self.0.clone())
    }
}

impl<R: Relation> fmt::Debug for Key<R>
where
    R::KeyValue: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key<{}>({:?})", R::NAME, self.0)
    }
}

impl<R: Relation> PartialEq for Key<R>
where
    R::KeyValue: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<R: Relation> Eq for Key<R> where R::KeyValue: Eq {}

impl<R: Relation> HostValue for Key<R> {
    fn to_value(&self) -> Val {
        Val::of_key(self.0.to_value())
    }

    fn from_value(v: &Val) -> Option<Self> {
        R::KeyValue::from_value(v.key()?).map(Key)
    }
}

/// Returns the next field of `fields` as a host value.
pub fn next<'v, T: HostValue>(fields: &mut impl Iterator<Item = &'v Val>) -> Option<T> {
    T::from_value(fields.next()?)
}

/// Returns `v`'s fields when it is constructor `ctor` of a declared sum.
pub fn ctor_fields(v: &Val, ctor: u32) -> Option<impl Iterator<Item = &Val>> {
    (v.ctor()? == ctor).then(|| Textual::fields(v))?
}

/// Returns the constructor index of `v` and its fields when it is a constructor value.
pub fn ctor_parts(v: &Val) -> Option<(u32, impl Iterator<Item = &Val>)> {
    Some((v.ctor()?, Textual::fields(v)?))
}

/// Returns constructor `ctor` of a declared sum, `fields` the header fields then the
/// constructor's own, each in declaration order.
pub fn of_ctor(ctor: u32, fields: Vec<(&str, Val)>) -> Val {
    Val::of_ctor(ctor, fields)
}

/// Returns an anonymous record, `fields` sorted by name.
pub fn of_record(fields: Vec<(&str, Val)>) -> Val {
    Val::of_record(fields)
}

/// Returns the fields of `v` when it is an anonymous record.
pub fn record_fields(v: &Val) -> Option<impl Iterator<Item = &Val>> {
    match v {
        Val::Record(fs) => Some(fs.iter()),
        _ => None,
    }
}

macro_rules! int {
    ($t:ty, $b:ident) => {
        impl HostValue for $t {
            fn to_value(&self) -> Val {
                Val::of_scalar(Scalar::Int(i64::from(*self)), Base::$b)
            }

            fn from_value(v: &Val) -> Option<Self> {
                match v.scalar()? {
                    Scalar::Int(i) => <$t>::try_from(i).ok(),
                    _ => None,
                }
            }
        }
    };
}

int!(i8, I8);
int!(i16, I16);
int!(i32, I32);
int!(i64, I64);

impl HostValue for f32 {
    fn to_value(&self) -> Val {
        Val::of_scalar(Scalar::F32(*self), Base::F32)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::F32(f) => Some(f),
            _ => None,
        }
    }
}

impl HostValue for f64 {
    fn to_value(&self) -> Val {
        Val::of_scalar(Scalar::F64(*self), Base::F64)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::F64(f) => Some(f),
            _ => None,
        }
    }
}

impl HostValue for String {
    fn to_value(&self) -> Val {
        Val::of_scalar(Scalar::Text(self), Base::Text)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::Text(t) => Some(t.to_owned()),
            _ => None,
        }
    }
}

impl HostValue for bool {
    fn to_value(&self) -> Val {
        Val::of_ctor(u32::from(*self), Vec::new())
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.ctor()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

impl HostValue for () {
    fn to_value(&self) -> Val {
        Val::of_record(Vec::new())
    }

    fn from_value(v: &Val) -> Option<Self> {
        record_fields(v)?.next().is_none().then_some(())
    }
}

impl<T: HostValue> HostValue for Option<T> {
    fn to_value(&self) -> Val {
        match self {
            None => Val::of_ctor(0, Vec::new()),
            Some(x) => Val::of_ctor(1, vec![("value", x.to_value())]),
        }
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.ctor()? {
            0 => Some(None),
            1 => next(&mut Textual::fields(v)?).map(Some),
            _ => None,
        }
    }
}

/// A set, its elements ascending in the model's order when the engine returns it.
impl<T: HostValue> HostValue for Vec<T> {
    fn to_value(&self) -> Val {
        Val::of_set(self.iter().map(HostValue::to_value).collect())
    }

    fn from_value(v: &Val) -> Option<Self> {
        v.elems()?.map(T::from_value).collect()
    }
}

macro_rules! scalar {
    ($(#[$doc:meta])* $name:ident($t:ty), $s:ident, $b:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $t);

        impl HostValue for $name {
            fn to_value(&self) -> Val {
                Val::of_scalar(Scalar::$s(self.0.into()), Base::$b)
            }

            fn from_value(v: &Val) -> Option<Self> {
                match v.scalar()? {
                    Scalar::$s(x) => <$t>::try_from(x).ok().map($name),
                    _ => None,
                }
            }
        }
    };
}

scalar!(
    /// Microseconds since 1970-01-01T00:00:00Z.
    Instant(i64), Instant, Instant
);
scalar!(
    /// Days since 1970-01-01.
    Date(i32), Date, Date
);
scalar!(
    /// Microseconds since midnight.
    Time(i64), Time, Time
);
scalar!(
    /// Microseconds since 1970-01-01T00:00:00, civil.
    DateTime(i64), DateTime, DateTime
);
scalar!(
    /// Microseconds.
    Duration(i64), Duration, Duration
);

/// A decimal of scale `S`: its unscaled digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal<const S: u8>(pub i128);

impl<const S: u8> HostValue for Decimal<S> {
    fn to_value(&self) -> Val {
        Val::of_scalar(Scalar::Decimal(self.0), Base::Decimal(S))
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::Decimal(d) => Some(Decimal(d)),
            _ => None,
        }
    }
}

macro_rules! bytes {
    ($(#[$doc:meta])* $name:ident, $b:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub Vec<u8>);

        impl HostValue for $name {
            fn to_value(&self) -> Val {
                Val::of_scalar(Scalar::Bytes(&self.0), Base::$b)
            }

            fn from_value(v: &Val) -> Option<Self> {
                match v.scalar()? {
                    Scalar::Bytes(b) => Some($name(b.to_vec())),
                    _ => None,
                }
            }
        }
    };
}

bytes!(Bytes, Bytes);
bytes!(SortKey, SortKey);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid(pub [u8; 16]);

impl HostValue for Uuid {
    fn to_value(&self) -> Val {
        Val::of_scalar(Scalar::Bytes(&self.0), Base::Bytes)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::Bytes(b) => b.try_into().ok().map(Uuid),
            _ => None,
        }
    }
}

/// A local date-time in a zone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ZonedDateTime {
    /// Microseconds since 1970-01-01T00:00:00, civil.
    pub local: i64,
    pub zone: String,
}

impl HostValue for ZonedDateTime {
    fn to_value(&self) -> Val {
        let s = Scalar::Zoned {
            local: self.local,
            zone: &self.zone,
        };
        Val::of_scalar(s, Base::ZonedDateTime)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::Zoned { local, zone } => Some(ZonedDateTime {
                local,
                zone: zone.to_owned(),
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Period {
    pub months: i32,
    pub days: i32,
}

impl HostValue for Period {
    fn to_value(&self) -> Val {
        let s = Scalar::Period {
            months: self.months,
            days: self.days,
        };
        Val::of_scalar(s, Base::Period)
    }

    fn from_value(v: &Val) -> Option<Self> {
        match v.scalar()? {
            Scalar::Period { months, days } => Some(Period { months, days }),
            _ => None,
        }
    }
}

/// The elements of a set-valued result, each a host value of type `T`.
#[derive(Debug)]
pub struct Rows<T, E> {
    rows: DynRows,
    of: PhantomData<fn() -> (T, E)>,
}

impl<T: HostValue, E: From<Error>> Iterator for Rows<T, E> {
    type Item = Result<T, E>;

    fn next(&mut self) -> Option<Self::Item> {
        let v = self.rows.next()?;
        Some(v.and_then(|v| host(&v)).map_err(E::from))
    }
}

/// The result of a registered transaction called inside an interactive transaction, committed
/// with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending<T> {
    pub value: T,
}

fn host<T: HostValue>(v: &Val) -> Result<T, Error> {
    T::from_value(v).ok_or_else(|| Error::Fatal {
        reason: FatalReason::Malformed {
            detail: format!(
                "a result {v:?} is no value of the host type {}",
                std::any::type_name::<T>()
            ),
        },
    })
}

fn only<T: HostValue>(mut rows: DynRows) -> Result<T, Error> {
    let v = rows.next().ok_or_else(|| Error::Fatal {
        reason: FatalReason::Malformed {
            detail: "a call of a result that is no set returned no value".into(),
        },
    })??;
    host(&v)
}

fn rows_of(c: Called) -> DynRows {
    match c {
        Called::Rows(rows) => rows,
        Called::Committed(done) => done.value,
    }
}

fn in_txn_rows(c: InTxn) -> DynRows {
    match c {
        InTxn::Rows(rows) | InTxn::Pending(rows) => rows,
    }
}

/// Returns the elements of a set-valued result.
pub fn rows<T, E>(c: Called) -> Result<Rows<T, E>, Error> {
    Ok(Rows {
        rows: rows_of(c),
        of: PhantomData,
    })
}

/// Returns the elements of a set-valued result of a call inside a transaction.
pub fn rows_in_txn<T, E>(c: InTxn) -> Result<Rows<T, E>, Error> {
    Ok(Rows {
        rows: in_txn_rows(c),
        of: PhantomData,
    })
}

/// Returns the one value of a result that is no set.
pub fn value<T: HostValue>(c: Called) -> Result<T, Error> {
    only(rows_of(c))
}

/// Returns the one value of a result that is no set, of a call inside a transaction.
pub fn value_in_txn<T: HostValue>(c: InTxn) -> Result<T, Error> {
    only(in_txn_rows(c))
}

/// Returns the result of a registered transaction with its commit.
pub fn committed<T: HostValue>(c: Called) -> Result<Committed<T>, Error> {
    match c {
        Called::Committed(done) => Ok(Committed {
            value: only(done.value)?,
            position: done.position,
            attempts: done.attempts,
        }),
        Called::Rows(_) => Err(Error::invalid(
            "a registered transaction returns its commit",
        )),
    }
}

/// Returns the result of a registered transaction called inside a transaction.
pub fn pending<T: HostValue>(c: InTxn) -> Result<Pending<T>, Error> {
    Ok(Pending {
        value: only(in_txn_rows(c))?,
    })
}

/// A bundle bound at the version it pins, by which its callables are called.
#[derive(Debug)]
pub struct Bundle<B, R: Runtime = adtb_sys::os::OsRuntime> {
    db: Database<R>,
    reg: Registered<R>,
    def: PhantomData<fn() -> B>,
}

impl<B, R: Runtime> Clone for Bundle<B, R> {
    fn clone(&self) -> Self {
        Bundle {
            db: self.db.clone(),
            reg: self.reg.clone(),
            def: PhantomData,
        }
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the handle of bundle `def`, binding to it and renewing its lease.
    ///
    /// Fail as [`Database::registered`] does, and with `VersionMismatch` when the registered
    /// bundle of that name pins another version.
    pub fn bundle<B: BundleDef>(&self, _def: &B) -> Result<Bundle<B, R>, B::Error> {
        let reg = self.registered(B::NAME)?;
        if let Some(pins) = reg.pins().filter(|&p| p != B::PINS) {
            return Err(Error::rejected(RejectReason::VersionMismatch {
                bundle: B::NAME.to_owned(),
                version: i64::try_from(pins).unwrap_or(i64::MAX),
            })
            .into());
        }
        Ok(Bundle {
            db: self.clone(),
            reg,
            def: PhantomData,
        })
    }
}

impl<B: BundleDef, R: Runtime> Bundle<B, R> {
    /// Call `c` with `args` as a transaction of its own.
    pub fn call<C: Callable<Bundle = B>>(
        &self,
        _c: C,
        args: C::Args,
        opts: &CallOptions,
    ) -> Result<C::Output, B::Error> {
        let called = self.reg.call(C::ORDINAL, &args.values(), opts)?;
        Ok(C::output(called)?)
    }

    /// Call `c` as [`call`](Bundle::call) does and return its result as Arrow record batches.
    pub fn call_arrow<C: Callable<Bundle = B>>(
        &self,
        _c: C,
        args: C::Args,
        opts: &CallOptions,
    ) -> Result<ArrowStream, B::Error> {
        Ok(self.reg.call_arrow(C::ORDINAL, &args.values(), opts)?)
    }

    /// Begin an interactive transaction whose calls name this bundle's callables.
    pub fn begin(&self, opts: &TxnOptions) -> Result<Transaction<'_, B, R>, B::Error> {
        Ok(Transaction {
            txn: self.db.begin(opts)?,
            reg: &self.reg,
            def: PhantomData,
        })
    }

    /// Run `f` in a new interactive transaction and commit it, running it again in a fresh
    /// transaction while it fails with a serialization failure, at most `attempts` times in all.
    ///
    /// Preconditions: `f` has no effect outside the database.
    pub fn transact<T>(
        &self,
        opts: &TxnOptions,
        attempts: u32,
        mut f: impl FnMut(&mut Transaction<'_, B, R>) -> Result<T, B::Error>,
    ) -> Result<Committed<T>, B::Error> {
        let mut attempt = 1;
        loop {
            let mut t = self.begin(opts)?;
            let value = match f(&mut t) {
                Ok(value) => value,
                Err(_) if t.txn.serialization_failed() && attempt < attempts => {
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            match t.txn.commit(None) {
                Ok(position) => {
                    return Ok(Committed {
                        value,
                        position,
                        attempts: attempt,
                    });
                }
                Err(e) if e.is_serialization() && attempt < attempts => attempt += 1,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// An interactive transaction whose calls name the callables of bundle `B`. Dropping it without
/// a commit rolls it back.
#[derive(Debug)]
pub struct Transaction<'a, B, R: Runtime = adtb_sys::os::OsRuntime> {
    txn: AdHocTransaction<'a, R>,
    reg: &'a Registered<R>,
    def: PhantomData<fn() -> B>,
}

impl<B: BundleDef, R: Runtime> Transaction<'_, B, R> {
    /// Call `c` with `args` as one call of the transaction.
    pub fn call<C: Callable<Bundle = B>>(
        &mut self,
        _c: C,
        args: C::Args,
        opts: &TxnCallOptions,
    ) -> Result<C::InTxn, B::Error> {
        let got = self.txn.call(self.reg, C::ORDINAL, &args.values(), opts)?;
        Ok(C::in_txn(got)?)
    }

    /// Run ad hoc text as one call of the transaction, as [`AdHocTransaction::query_text`] does.
    pub fn query_text(
        &mut self,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: &TxnCallOptions,
    ) -> Result<Vec<DynRows>, Error> {
        self.txn.query_text(text, params, opts)
    }

    /// Commit the transaction. Returns its commit position.
    pub fn commit(self, durability: Option<Durability>) -> Result<Position, B::Error> {
        Ok(self.txn.commit(durability)?)
    }

    pub fn rollback(self) -> Result<(), B::Error> {
        Ok(self.txn.rollback()?)
    }
}
