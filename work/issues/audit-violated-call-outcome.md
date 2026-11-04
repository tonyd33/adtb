---
created-at: 2026-11-04
status: open
kind: bug
priority: 470
summary: "A Violated call's audit row records Internal, not the error returned"
---

# Audit row of a Violated call holds Internal

## Report

A call refused with `Violated` writes its `catalog::audit` Call row as `Failed { error: Rejected { reason: Internal { what: "...Violated..." } } }`. security.md says `Failed` carries the error value returned, so the row should hold the `Violated` value.

Cause: the error reaches the audit writer as text. `Tables::error_value` in crates/adtb/src/api/server/tables.rs parses it back, and the `report: N rows` part of a `Violated` value does not read back, so it falls back to `Internal`.

Found while moving the server tests onto engine scripts (2026-11-04).

## Proposal

Pass the error value itself to the audit writer instead of its text. Any error whose text does not read back has the same fault; a test should write each refusal kind through a call and compare the audit row with the returned value.
