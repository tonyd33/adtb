---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 745
summary: "a path index or unique has no B-tree: the planner never uses a path index, and a path unique is checked by scan"
closed: 2026-10-26
---

# Path index btree

## Report

## Report

`crates/adtb/src/catalog/rows.rs` `IndexSpec::stored` keeps path indexes out of the B-trees. A path unique is enforced only by the scan in `check_uniques`, which is outside the index-bounded rule, and the planner cannot probe a path index. Build B-trees over field paths into value-level sums, holding only rows whose path exists.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as pyryqsnm. The engine failing list of the conformance suite is now empty.
