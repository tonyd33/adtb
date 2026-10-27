---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 802
summary: "migration gaps left by the operator log: nested add field, internal errors as refusal strings, no internal open error, add then rename, validation memory, rekey revert"
closed: 2026-10-26
---

# Operator log follow ups

## Report

## Report

From the operator log tracks:
- `add field` on a type used as a nested value in another relation's field is not mapped; `convert` refuses it.
- An internal error inside a fill or routing is reported as a refusal string, because `change::Fill` returns `String`.
- `catalog::Error::Internal` reaches users as `OpenError::Corrupt`; `OpenError` has no internal variant.
- `add field X` then `rename field X` in one migration is refused by `Source::resolved` ("gone by the migration's end").
- Validation evaluates constraints in full with a fixed 1 GiB grant (`VALIDATION_MEMORY` in `crates/adtb/src/catalog/validate.rs`), whatever the relation's size.
- Revert of a rekey is untested.

## 2026-10-26: internal open error

`catalog::Error::Internal` during a migration now reaches users as `MigrationRefused { reason: Rejected { Internal { what } } }`, since the spec lists `OpenError` exactly; no new variant.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrtsvsxs.
