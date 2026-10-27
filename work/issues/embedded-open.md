---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 830
summary: "an embedded database cannot be opened with its bundles, and host functions and bulk append do not exist"
closed: 2026-10-27
depends-on:
  - bundles
  - in-process-api
tags:
  - engine
---

# Feature: embedded open steps, host functions and bulk append

## Report

Implement `docs/spec/embedding.md` §2 to §4, §7 to §10 and
`docs/spec/interop.md` §9: the embedded open steps with bundles,
calls, cancellation, configuration, bulk append, host functions.

Prerequisites: `bundles.md`, `in-process-api.md`.

## 2026-10-27: done

Landed, with tests in `crates/adtb-sim/tests/embedding.rs`, `loads.rs`, `host_fns.rs` and `api_real_file.rs`, and contracts in `docs/design/server.md` §4.1 to §4.4:

- Cancellation: `CancelToken` in `CallOptions`, `TxnCallOptions` and `LoadOptions`; a cancelled call has no effect and leaves an interactive transaction open; a running query stops at its next morsel boundary; a cancellation racing a commit either commits or has no effect.
- `deadline` and `limit` on calls and interactive transactions: `DeadlineExceeded` and `StatementLimit { Time }`.
- Close waits for running calls and closes the database for every clone and registration.
- Arrow results: `Registered::call_arrow` returns an `ArrowStream` of `RecordBatch`es (engine `batch::Batch` columns under a named `ArrowSchema`, sums carrying constructor ids), mapped as the wire's Arrow format maps result types.
- Bulk append: `Database::load`, `Load::appender("r.C")`, `Appender::push`, `Load::commit`, one change set at commit, `SchemaMismatch` at push, NFC text, all or nothing across a crash.
- Host functions: `OpenOptions::host_fns` with `HostFns::register`; checked before migrations and after them (`HostFnMismatch` for version, determinism or unmappable types, a shipped `redefine fn` admitted; `HostFnMissing` when a stored part reaches it); called once per batch from queries, write values and constraints; an error, panic or ill-shaped result faults with `FunctionFailed`; a call reaching a missing one fails with `MissingFunction`.
- `sys::process_options` shows the open-time settings read-only; a write targeting a volatile relation is refused.
- The file setting `RetryLimit` replaces the constant retry limit.

Still open:

- Results are materialised, not streamed; dropping a result cancels nothing.
- Typed `Bundle<B>`, `Callable`, `BundleDef` and generated error and notice types (ticket `codegen`).
- Feeds (§4.4) and notices (§4.5) in process.
- Encryption and `KeySource` (`KeyRequired`, `WrongKey`), `FormatTooNew`, `finalize_upgrade`, and the downgrade cause for a host function redefined since a bundle's pin.
- Open-time `temp_dir`, `archive`, `on_metrics`, and defaults derived from the host (pool at 25% of memory, one worker per core).
- `priority`, `trace` and `Quorum` durability are accepted and ignored.
- Host functions are not bound while migrations validate or build indexes, nor in index key and expression evaluation (`catalog/keying.rs` builds `exec::Scalar` with no bodies); a staged job reaching a host function before its `redefine fn` is not refused with `HostFnMismatch`.
- A `Volatile` host function is not held to one evaluation per call in a query's meaning.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the embed track. What stays open is in `embedded-open-follow-ups`, `host-fns-in-keys`, `spec-closed-database-reason` and `conformance-refused-schema`.
