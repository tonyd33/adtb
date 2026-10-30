---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 300
summary: Every open panics after store of a view whose rows are Set Text
closed: 2026-10-29
---

# Every open panics after store of a view whose rows are Set Text

## Report

Found by the manual-gaps track on 2026-10-29. After the migration
`define fn cats () -> Set Text = from c in pets.Cat select c.name` followed by `store cats`,
every open panics at crates/adtb/src/catalog/rows.rs:596: "the rows of stored view `cats` have
a type id". adtb-shell exits 101. Record and named element types work. The database cannot be
opened again. Fix: give a stored view of a scalar element type its row type, or refuse the
`store` at apply; add a test that reopens.

## 2026-10-29: done

The panic happened during apply, before commit, so the log never held the migration: the file opened at its previous version and panicked when the migration was offered again. A file left by the panicking binary applies it under the fix. Every stored view of a `Set` whose element is not a declared type now gets a row type. `store` or `redefine fn` of a stored view whose result is not a `Set` is refused (spec/store-non-set/refuse, derived.md 2.1). Landed at 666f0892; branch check 2877 passed, 7 skipped, campaign 52. Tests in crates/adtb-sim/tests/oplog.rs and api_real_file.rs.

## 2026-10-29: resolved
