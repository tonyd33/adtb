---
created-at: 2026-11-12
status: open
kind: bug
priority: 330
summary: "the revert dependency gate misses the key fields of a later rekey, so the revert fails in its inverse instead"
tags:
  - class-identity-by-spelling
---

# The revert gate misses a later rekey's key fields

## Report

`add type W = { id: I64 }`, `add relation ws : W key id`; then `add field W.k: Text fill "k"` (0002); then `rekey relation ws key k` (0003). `revert 0002` is not refused as a dependant; it fails in its inverse with "key field `k` is not a header field or a field of the only constructor of `W`". evolution.md §5.4 wants "`revert 0002` is refused while migration 0003 depends on it: it names field `auth::W.k`". Cause: rekey writes its key clause without copy marks, and `Resolved::Rekey` holds the fields as text, so the gate's (space, id) sets miss them. Found in structure-lang-identity step 5c-2. Same on main.

Direction: `Resolved::Rekey` holds field ids, and the gate reads them (structure-lang-identity). Test: the case above with `refused_as`.

Defect class: Catalog rows and identity by position or spelling.
