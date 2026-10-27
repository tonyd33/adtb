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
    AdHocTransaction, CallOptions, Called, Committed, CreateOptions, Database, Durability, DynRows,
    Error, FatalReason, Fault, InTxn, Limit, Log, OpenError, OpenOptions, Position, Registered,
    Registration, RejectReason, RetryReason, TxnCallOptions, TxnId, TxnOptions,
};
pub use exec::Val as Value;
