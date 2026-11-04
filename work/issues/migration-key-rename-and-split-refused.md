---
created-at: 2026-11-03
status: resolved
closed: 2026-11-04
kind: bug
priority: 300
summary: "a migration is wrongly refused when renaming a composite key field reorders the key, or when splitting a constructor a stored key row holds"
---

# Migrations refused over composite key rename and split in a key

## Report

Found by the translate-reach track, reproduced there (receipt work/briefs/receipt-translate-reach-r1.md, local).

- Renaming a composite key field so that the key's sorted field order changes (prices keyed `(sku, n)`, `rename field Price.sku to code`) refuses the migration: "a value stored as Scalar(I64) does not convert to Scalar(Text)". Cause: crates/adtb/src/catalog/change.rs `convert` matches key fields by position. Same class as pinned-composite-key-order: identity by position in the key record.
- Splitting `Tier.Paid` when a stored row of `p`, keyed `(t: Tier, n)`, holds `Paid` in its key refuses with "p: a stored value holds constructor 1026 ...". Seen with a live bundle; not isolated without one.

Both are wrong refusals: no data changes and nothing wrong is returned, but a valid migration cannot apply.

## 2026-11-04: fixed

`fix: convert stored key values by field identity across a rename and a split`. Rename: stored key fields were paired by position, and copied when the field types were equal, so a rename that changes the key's sorted order paired the wrong fields; every kept field now converts through `change::stored`, matching data and key fields by catalog id, and a single key field with another id refuses. A rename that reorders a key's fields of one type left the relations holding that key unrewritten, so their references stopped resolving (a wrong answer on main); `Shape` now carries the key field ids of every relation reached through `Key`, directly, in an `Option` or inside another key. Split: values of the split type held in other relations' fields were never routed, with or without keys or a bundle; `Moved.routes` routes them as evolution.md §1.3 asks. Class: identity by position or spelling. Search test `key_renames` over generated composite keys of 2 to 4 fields; scripts for the split held in a key, beside a live bundle, a deeply held key and a rekey of a wrapped reference. Review found no blocker. Full check 3380 passed.

Left, on migration-identity-sweep: constructors found by name across schemas, merge, nest and unnest of a held value, `Tag T` on a split, a rekey carrying a wrapped reference, per-row layout cost, the sorted-key-order rule outside catalog.
