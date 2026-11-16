---
created-at: 2026-11-15
status: resolved
closed: 2026-11-15
kind: bug
priority: 200
summary: "a kept verify finding with a valid `Finding` id and the wrong fields is read as a malformed value, not refused as damage"
tags:
  - quality
  - engine
  - class-damage-unreported
---

# A kept verify finding with the wrong fields is read as a malformed value

## Report

Found by the review of structure-lang-identity step 9c (nlzuqomn c85d1afb). `Findings::of` (crates/adtb/src/api/verify.rs) checks a finding row's constructor id and its first two header fields (`id`, `run`), but not the field count or kinds against the constructor's declared fields. `val` builds `Val::ctor(position, fields)` from whatever the row holds, so a row of `Order` with one `Text` field, or an `Int` where a `Text` belongs, enters `sys::verify_findings` as an ill-typed value. Same before 9c, with positions instead of ids.

Fix direction: check each kept field against the constructor's header and own field types from the schema at open, and refuse a mismatch as `Corruption::Finding`; test with a row of each mismatch.

## Resolved

Landed as wsmstooz 97cc33d4 (structure-lang-identity step 11 part d, 2 rounds, full check 0, 3839 tests). Kept verify findings read through `FindingRow`, a `Decode` of `sys::verify_findings` by `FINDING_*` fixed ids; the writer emits a `Kept` of field handles, so writer and reader share one layout; counters and cursor rows must be two integers of no constructor. A row of another shape is `Corruption::Finding(KeptRow, Rot)` at open, its whole reason in the detail. Read-back runs under the shipped, constructor-reversed and field-reversed catalogs; 14 refusals asserted whole.
