---
created-at: 2026-10-26
status: open
kind: bug
priority: 793
summary: "index path keying and derived elements panic on a checker or stored-row guarantee instead of failing with an internal error, and dropping a transaction can panic while unwinding"
tags:
  - quality
---

# Index keying and transaction drop panic instead of failing with an internal error

## Report

`crates/adtb/src/catalog/rows.rs` `path_steps` (rows.rs:260) panics at rows.rs:262, :268 and :295 when a path names no field or constructor, and `at_path` (rows.rs:304) panics at :306 and :316 when a stored value does not have the shape the path's schema gives it. The first is a checker guarantee and the second a stored-row guarantee; both run on the write path and at migration, where the catalog already has `Error::Internal` and `Error::Corrupt`. `crates/adtb/src/catalog/keying.rs:165` panics when a derived element has no normal program, an invariant of `index_program` (`crates/adtb/src/catalog/mod.rs:635`, :953) that `Option<&NormalProgram>` does not express. `crates/adtb/src/write/eval.rs:593` panics when an upsert names a unique its relation lacks.

`crates/adtb/src/api/txn.rs:213` asserts inside `Drop for AdHocTransaction` that an abort fails only with a storage fatal. Dropping a transaction while a caller's code is unwinding and the abort fails otherwise panics twice and aborts the process.

Fix: have `path_steps`, `at_path` and `IndexKeys::new` return a `Result` with an internal error naming the guarantee (corrupt for `at_path`), and pass the normal program as required wherever an index has a derived element; return an internal error at eval.rs:593. Skip the assertion in `Drop` when `std::thread::panicking()`.
