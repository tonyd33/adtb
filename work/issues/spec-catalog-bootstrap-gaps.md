---
created-at: 2026-10-25
status: open
kind: bug
priority: 294
summary: "catalog positions, the Tag type's row, value_states coverage, key clause text and relation kinds are unstated"
tags:
  - spec
---

# Bug: Bootstrap catalog gaps

## Report

From the bootstrap catalog. My rulings:
1. `position` of a constructor or field counts from 1, as `ordinal`
   does (`catalog.md` §2.1). The catalog code uses 0 today; change it.
2. `Tag` (id 9) is declared in the engine's declarations and has a
   `catalog::types` row like every other object.
3. `value_states` has a row for every constructor of every declared
   sum a field of any declared type reaches, prelude sums included.
4. A key clause's `clause` text is `key f` or `key (a, b)`, as if
   written.
5. `catalog::` relations have kind `Catalog`, and `sys::` relations
   `Volatile`.

State each in `catalog.md`.
