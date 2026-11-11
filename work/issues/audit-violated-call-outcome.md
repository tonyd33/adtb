---
created-at: 2026-11-04
status: open
kind: bug
priority: 470
summary: "A Violated call's audit row records Internal, not the error returned"
tags:
  - class-identity-by-spelling
---

# Audit row of a Violated call holds Internal

## Report

A call refused with `Violated` writes its `catalog::audit` Call row as `Failed { error: Rejected { reason: Internal { what: "...Violated..." } } }`. security.md says `Failed` carries the error value returned, so the row should hold the `Violated` value.

Cause: the error reaches the audit writer as text. `Tables::error_value` in crates/adtb/src/api/server/tables.rs parses it back, and the `report: N rows` part of a `Violated` value does not read back, so it falls back to `Internal`.

Found while moving the server tests onto engine scripts (2026-11-04).

## Proposal

Pass the error value itself to the audit writer instead of its text. Any error whose text does not read back has the same fault; a test should write each refusal kind through a call and compare the audit row with the returned value.

Defect class: Identity by spelling: a string or position stands in for the identity the type system could hold.

## 2026-11-11

structure-transactions step 13 builds every audit row's error from the `api::Error` itself (no text round trip), which fixed the rows of `Serialization`, `TooOld`, `HistoryUnavailable`, `PositionNotOnHistory` and `RestorePointUnavailable`. `Violated` is the one reason left: its `report: Report` is `new Bytes`, a ResultType descriptor plus a Rows body (catalog.md §4, wire.md §5.2), and the engine's `RejectReason::Violated` holds only untyped `Vec<Val>` with no encoder, so the row still records `Internal` through an explicit `NoValue::Report` arm. A test asserting the required row is `#[ignore]`d until this lands.

Remaining work: carry the constraint's result type with the error, encode the report bytes, and rule first on security.md §6, whether the audit reader may see report rows only the caller could read.
