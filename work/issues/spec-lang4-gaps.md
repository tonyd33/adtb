---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 293
summary: "spec gaps from file scans, temp relations, aggregates and id assignment"
branch: adtb-spec3
tags:
  - spec
---

# Bug: Gaps from the catalog id and corpus work

## Report

My rulings:

1. `interop.md:411` uses `rejects into identity_rejects` with nothing
   declaring it. The block declares its own temp relation.
2. `interop.md:507` depends on block 467's temp relation. Restate what
   it needs in the block, and drop the harness's `CONTINUES` table.
3. An aggregate takes exactly one parameter; several values pass as a
   record. State it in `derived.md`; a declaration with more is
   refused.
4. `store f` creates a wrapper type only when the view's element type
   is anonymous. A view of a nominal type is a relation of that type.
   Amend `catalog.md` §1.2's `store f` row and the `ids1` ruling's
   wording.
5. A move down keeps the id on the first listed constructor in the
   type's declaration order. State it.
6. `unnest` re-scopes indexes as split, merge and nest do. An index
   with no `in` is not re-scoped and keeps its id. State it.
7. Prelude functions have no catalog id and no row: the prelude is not
   part of a schema. State it.
8. A temp relation may not take any name a module the text reads
   declares: relation, function, type or constructor.
9. A temp relation of a sum with several constructors must declare its
   key. Change the failing test
   `a_temp_relation_of_a_sum_without_a_key_is_keyed_by_its_whole_row` to
   expect the refusal, and state it in `interop.md` §4.3.
