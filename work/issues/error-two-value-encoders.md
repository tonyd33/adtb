---
created-at: 2026-11-11
status: open
kind: bug
priority: 500
summary: "api::Error has two encoders of the prelude Error sum, and Error::value()'s text differs from the prelude's syntax for six reasons"
tags:
  - class-copied-rules
---

# Two encoders of one error sum

## Report

## 2026-11-11

Since structure-transactions step 13, `Error::prelude_value` (crates/adtb/src/api/error_value.rs) builds an `api::Error` as a `prelude::Error` value from the schema's type definitions, and `Error::value()` (api/error.rs) prints the same sum as text by its own rules. Their outputs differ for `Serialization` (`txn: 7`, not `TxnId(7)`), `TooOld`, `HistoryUnavailable`, `PositionNotOnHistory`, `RestorePointUnavailable` (`{ epoch, lsn }`, not `Position { .. }`) and `Violated` (`report: N rows`). `value()` is public and used across crates: the sim's script driver (`failed` and `error` directives), execution/longkey.rs, maintenance/reindex_review.rs, and `Broken::NoValue`'s Display; about ten scripts compare its `Violated` text. A new reason or field changed in one encoder and not the other diverges silently.

Direction: one encoder. Print `prelude_value` through the prelude form wherever a schema is at hand (the script driver has one), keep `value()` only where none is and state what it is, or give `Violated`'s report a value form first (audit-violated-call-outcome) and then retire `value()`; move the scripts to the prelude text.

Defect class: Copied rules: one rule kept in two places that drift apart.
