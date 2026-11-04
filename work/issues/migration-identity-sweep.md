---
created-at: 2026-11-04
status: open
kind: bug
priority: 380
summary: "Migration conversion still finds constructors by name across schemas, falls back to field names, and keeps seven copies of the sorted-key-order rule; merge, nest and unnest of a held value refuse as split did"
---

# Identity by name and position left in migration conversion

## Report

The migration-key track fixed stored key fields matched by position (`change::stored`, `keyed`, `by_id`) and values of a split type held in other relations' fields (`Moved.routes`). Its sweep of crates/adtb/src/catalog/change.rs and moved.rs left these, each the same class (identity by position or spelling) or the same gap, with no wrong answer shown yet:

- `ctor_named(b, .., ctor_name(a, ..))` finds a constructor by name across two schemas in fills, copies, route and key_value.
- `carry` falls back to a field's name when the field has no id.
- Merge, nest and unnest of a type held as a value in another relation's field refuse as split did before `held_routes`.
- A split refuses parametric types and `Tag T` held as values.
- Seven copies of the sorted-key-order rule outside catalog: crates/adtb/src/api/pinned.rs, load.rs, exec/types.rs, exec/store.rs, write/schema.rs, write/fresh.rs, write/eval.rs. `layout::key_names` is now the rule inside catalog.

Details are in the track's local receipt, work/briefs/receipt-migration-key-r1.md.

## Proposal

Route every lookup across schemas through catalog ids, and make `layout::key_names` the only sorted-key-order rule, so a disagreement cannot compile. Give merge, nest and unnest the same held-value routing as split.
