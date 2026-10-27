#![forbid(unsafe_code)]

mod api;
pub mod batch;
pub mod btree;
pub mod catalog;
pub mod exec;
pub mod heap;
pub mod history;
pub mod key;
pub mod mvcc;
pub mod plan;
pub mod storage;
pub mod vacuum;
pub mod verify;
pub mod write;

pub use adtb_sys::runtime;

#[cfg(target_os = "linux")]
pub use api::open;
pub use api::{
    AdHocTransaction, Appender, ArrowSchema, ArrowStream, CallOptions, Called, CancelToken,
    Committed, CreateOptions, Database, Determinism, Durability, DynRows, Error, FatalReason,
    Fault, Field, HostFns, InTxn, Limit, Load, LoadOptions, LoadReport, Log, OpenError,
    OpenOptions, Position, RecordBatch, Registered, Registration, RejectReason, RetryReason,
    TxnCallOptions, TxnId, TxnOptions,
};
pub use api::{BundleDef, Callable, CtorRelation, Key, Pending, Rows, typed};
pub use exec::HostError;
pub use exec::Val as Value;
