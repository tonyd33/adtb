---
created-at: 2026-10-25
status: open
kind: feature
priority: 780
summary: "the in-process API has no read-only deferrable path, call options, engine transaction ids or Display for errors, and leaks a page on one crash"
tags: [engine]
---

# Feature: in-process API follow-ups

## Report

From the in-process API track:
- `write::Database::begin` starts only read-write transactions, so a deferrable read-only transaction is refused (`api::a_deferrable_read_only_transaction_reads_and_commits` fails).
- `CallOptions` and `TxnCallOptions` have no fields (deadline, limit, priority, cancel, assert, trace, retry); `commit` takes no durability; `OpenOptions` has no key, archive, host functions or metrics, and fixed default sizes. The retry limit is fixed at 5 until `catalog::settings` exists.
- A serialization failure carries the xid as `txn` and `""` as `on`; nothing is written to `sys::conflicts`. `Locked` never carries a pid, and `SchemaRefused.operator` is always `""`.
- A violation report comes back as untyped `Vec<Val>` with `redacted` 0, not the spec's wire-encoded `Report`.
- A crash between allocating the counters page and naming it in the root page leaks one page.
- A failed result-shape check in `Prepared::results` ends an interactive transaction instead of failing only the call.
- `Drop` for `AdHocTransaction` (`api/txn.rs`) discards the abort result with `.ok()`.
- `close(self)` drops one handle; the engine closes when the last clone drops. Opening one file twice in a process is caught only on the OS path, not through `open_in`.
- `api::Error` has no `Display`.
- Spec gap: no field of the error sum names which item of an ad hoc text failed.
- The contracts of `statements`, the commit position, the counters-page naming and `Catalog::into_bindings` are stated only in doc comments; the write path and catalog have no design file.
