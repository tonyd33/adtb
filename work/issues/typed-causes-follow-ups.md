---
created-at: 2026-11-03
status: open
kind: bug
priority: 640
summary: "after typed causes: two Corrupt causes that are internal, OpenError::Corrupt still text, exec's quoted Debug, write naming api::Error, and Display tests that miss an unlinked case"
---

# Typed causes follow-ups

## Report

Carried from typed-causes-engine-layers. None changes an answer or loses data.

- `write::Rot::NoKeyIndex` (write/schema.rs) reads as a broken invariant, not stored damage; moving it to `write::Broken` turns Fatal Storage into Rejected Internal. `catalog::Rot::KeptIndexes` and `IndexKeying` wrap Broken, Internal or Refused causes and report Corrupt at load. Each is an api outcome change: decide per site.
- `OpenError::Corrupt { detail: String }` is still text; `a_failed_read_of_the_test_database_records_no_verdict` in crates/adtb-sim/tests/backup.rs is ignored for it.
- `exec::Broken`'s Debug prints its Display quoted, kept because other `{e:?}` paths of exec errors rely on it; derive it once those paths print the text.
- `write::Broken::RefusalUnrecorded` and `WitnessDenialUnrecorded` hold `api::Error`, a layer above write.
- Relation names held as `String` where the site has a `RelId`, since Display has no schema: `UnstoredIndex`, `NotAnIndex`, `UniqueUnindexed`, `UndeclaredIndex`, `KeyNotFields`, `ReferenceUnindexed`, `write::Rot::NoKeyIndex`.
- The Display tests' `after` chains force an arm per variant but not that an arm is reached: an unlinked arm passes. No test reads `exec::Broken`'s Display.
- No test pins the text of a write cause inside `Corruption::Write`, the reindex failure cause or verify's details.
- `an_error_with_no_value_reads_as_its_value_text` (api/cause.rs) uses an error that has a value: its name overclaims. `a_primitive_of_mistyped_operands...` accepts any operand as `found`.
- `exec::Broken::OtherVersion` is a deployment binding mismatch under docs that call every variant a broken invariant.
