---
created-at: 2026-11-04
status: resolved
kind: bug
priority: 380
summary: "Migration conversion still finds constructors by name across schemas, falls back to field names, and keeps seven copies of the sorted-key-order rule; merge, nest and unnest of a held value refuse as split did"
closed: 2026-11-06
tags:
  - class-identity-by-spelling
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

## 2026-11-04: from the review of the migration-key change

- A rekey with `carry` of a reference wrapped in `Option` (or another library sum) is accepted by the carry check, but `Mapping::rekey` (crates/adtb/src/catalog/moved.rs) fills only fields typed `Key r` directly. The change makes it refuse; carrying it should map the old key to the new one through the wrapper.
- A split of a type held as `Tag T` still refuses ("a stored value holds constructor ... of a sum that no longer has it"): `held_routes` covers no tags. Route them to a field-less constructor of the routing's result.
- Each `Key` field of each rewritten row recomputes both schemas' key layouts, and each held sum walks `layout::ctors`. Hoist the per-relation layouts out of the per-row loop.
- api/pinned.rs keeps its own key order path for translation; whether it agrees with `layout::key_names` across a reorder is unchecked.

## 2026-11-04: change 1, ids across schemas and one key order rule

`fix: match constructors across schemas by id and keep one sorted key order rule`. Class: identity by spelling. `fills`, `copies`, `held_fills` and `key_value` find constructors (and key fields) by catalog id (`type_ctor_id`, `type_ctor_index` in catalog/ids.rs); `carry` no longer falls back to a field's name (`Broken::NoFieldId`). `route`, `held_routes`, `merge`, `nest`, `unnest` and the field a `move field` copies stay by name, as their contract states them. No user text reaches a case where the name lookup was wrong: every row-moving operator builds its source, so a step never spans a rename (a step that does not build and moves no rows folds, but a following map step is then refused, forward and on revert); the six by-name mutants survive, and the id types are the guard. One sorted key order rule, `adtb_lang::textual::key_names`, called by the nine former copies in adtb and adtb-lang and three in adtb-file, adtb-codegen and adtb-testkit; a search test (`catalog::tests::only_key_names_sorts_the_fields_of_a_key`) over every crate depending on adtb-lang flags any sort form or ordered collection near a relation's key, and fails with the probe copy put back. Tests: oplog/key-reorder-reads-and-writes.test, a load across a reorder, two reorder operators in pinned-keys; a mutant at each of the 11 call sites fails a test. Review found the first search test missed `sort_by` copies; fixed. Full check 3350 passed.

Also seen: a refusal of an operator over a source that does not build reports `operator: ""`.

Left for change 2: merge, nest and unnest held-value routing, `Tag T` in splits, parametric held values, `carry` through `Option`, per-relation layouts hoisted out of the row loop.

## 2026-11-06: change 2 landed

`fix: route held values through merge, nest, unnest and parametric splits, split tags and rekey through Option`. Merge without `by`, nest and unnest of a type held as a value route it through `held_merges`, `held_nests`, `held_unnests` into `Moved.routes`, keyed by (type id, constructor id); a replaced constructor with no route is still refused. Routes are makers instantiated at each value's type arguments, so parametric held values split, merge and nest; an instantiation whose arguments reach the type itself (`Box (Box I64)`) is refused by `unreached`. A `Tag T` of a replaced constructor is routed when the routing sends every value of it to one product (`sole_product`, header fields or not); otherwise refused naming the constructor. A rekey carrying `Option (Key r)` rewrites through the wrapper (`Refs`). Conversion plans are built once per (relation, constructor), not per row. adtb-lang's `Map::key_order` was an eleventh copy of the key order rule; it now calls `key_names`. Tests: oplog scripts held-values-structural, split-held-tag, split-held-tag-header, split-held-parametric, merge-held-value-in-key, unnest-held-value, rekey-carries-wrapped-reference, exact rows forward and on revert; the forward shapes were refused on main. Review found no blockers; its wrong messages, raw ids in the tag refusal and test gaps fixed. Production +709 -238 against an estimate of 250. `check --changed`: one unrelated failure under load (filed real-file-verify-stale-page); full `check` on main after landing.

Filed from it: held-value-routing-gaps (merge and nest of `Tag T` refused until the spec states an image, add field and nest over parametric held values), rekey-stale-held-key.

## 2026-11-06: resolved
