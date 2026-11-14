---
created-at: 2026-11-13
status: open
kind: bug
priority: 450
summary: "an operator's field replacements are discarded when the source after it does not build, so a clause a move down keeps takes a new catalog id when an earlier operator of the migration leaves the source unbuilt"
tags:
  - lang
  - quality
  - class-identity-by-spelling
---

# A move down after an unbuilt step renews a kept unique's id

## Report

`reconcile_clause_ids` (crates/adtb-lang/src/evolve/ids.rs:68) takes `self.replaced` before it checks that the source builds, and returns when it does not. The old-to-new field map of a move, nest, unnest or merge applied while the source is unbuilt is lost. `replacing` (ids.rs:109) also overwrites the map instead of adding to it. `created` and `dropped` survive unbuilt steps, but `replaced` does not. So whether a kept clause keeps its id depends on an unrelated earlier operator of the same migration.

Repro (reproduced as a temporary test in evolve_ids_tests.rs):

```
module auth
type T { id: Key ts, n: Text } = A { z: Text, w: Text } | B { z: Text }
relation ts : T
  key id generate sequence
  unique (n) in B
  index (w) in A
```

Index ids of `ts`: `[1025, 1026]`. After `move field T.n to T.(A | B)` alone: `[1025, 1026]`, as `a_move_down_keeps_the_id_of_a_unique_over_the_moved_field_in_a_constructor_it_lists` requires. After one migration `drop field T.A.w` / `move field T.n to T.(A | B)` / `add field T.A.w: Text fill "q"`: `[1027, 1028]`. The index over `w` renewing is the ruled behaviour for a re-added field. The unique over `n` renewing is not, since `n` was only moved.

Fix direction: keep the replacements of every operator since the last reconcile over a built source, composing each map through the earlier ones, and clear them only after a reconcile that ran.

Defect class: class-identity-by-spelling
