---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 300
summary: Every open panics after store of a view whose rows are Set Text
branch: view-panic
---

# Every open panics after store of a view whose rows are Set Text

## Report

Found by the manual-gaps track on 2026-10-29. After the migration
`define fn cats () -> Set Text = from c in pets.Cat select c.name` followed by `store cats`,
every open panics at crates/adtb/src/catalog/rows.rs:596: "the rows of stored view `cats` have
a type id". adtb-shell exits 101. Record and named element types work. The database cannot be
opened again. Fix: give a stored view of a scalar element type its row type, or refuse the
`store` at apply; add a test that reopens.
