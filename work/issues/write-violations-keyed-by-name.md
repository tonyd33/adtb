---
created-at: 2026-11-14
status: open
kind: bug
priority: 420
summary: "the write path collects constraint violations in a map keyed by the constraint's printed name, not its catalog id"
tags:
  - engine
  - class-identity-by-spelling
---

# Write violations are keyed by constraint name

## Report

crates/adtb/src/write/eval.rs:64 `pub type Violations = BTreeMap<String, Violation>`, filled by `violate_key` and `violate` with the constraint's name. Today no two live constraints share a name (a re-add under a pending drop's name is refused: pending-drop-ended-by-redeclared-name), so nothing merges, but identity rests on that refusal and on the spelling. Found in the work on readded-constraint-skips-validation; not reproduced.

Direction: key by `CatConstraintId` and print the name only when the refusal is built.

Defect class: Catalog rows and identity by position or spelling.
