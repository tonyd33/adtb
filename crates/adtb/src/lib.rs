//! The engine: catalog, planner, executor, transactions and storage.
#![forbid(unsafe_code)]

mod api;
pub mod batch;
mod btree;
mod catalog;
mod exec;
mod heap;
mod history;
#[cfg(test)]
mod history_tests;
pub mod key;
mod mvcc;
mod plan;
mod storage;
#[cfg(test)]
mod test_world;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod vacuum;
mod verify;
mod write;

pub use adtb_sys::runtime;

pub use api::{
    AdHocTransaction, Appender, ArrowSchema, ArrowStream, CallOptions, Called, CancelToken,
    Capability, Committed, CreateOptions, Database, Determinism, Durability, DynRows, Error,
    FatalReason, Fault, Field, HostFns, InTxn, Limit, Load, LoadOptions, LoadReport, Log,
    OpenError, OpenOptions, Position, RecordBatch, Registered, Registration, RejectReason,
    RetryReason, RoleAuthority, RoleSession, TxnCallOptions, TxnId, TxnOptions,
};
pub use api::{
    ArchiveTarget, Proof, RestorePoint, Restored, proof_in, prove_in, restore_in, restore_report_in,
};
pub use api::{
    Binding, CallDescriptor, CallKind, Completion, Format, Layout, Observation, Pins, ResultBatch,
    RoleBinding, Task, TaskOptions,
};
pub use api::{BundleDef, Callable, CtorRelation, Key, Pending, Rows, typed};
#[cfg(target_os = "linux")]
pub use api::{VerifyFileError, open, prove, restore, restore_decrypted, verify_file};
pub use catalog::{Dormant, version_hash};
pub use exec::HostError;
pub use exec::Val as Value;
pub use storage::Passphrase;
pub use write::{CheckPlan, Part as DeltaPart};
