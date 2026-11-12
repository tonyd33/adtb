---
created-at: 2026-11-12
status: resolved
kind: bug
priority: 360
summary: "a unique or index element is keyed by one resolved copy of its field, so a move down or a rename under a set scope gives the clause a new id"
closed: 2026-11-12
tags:
  - class-identity-by-spelling
---

# Clause elements keyed by one copy of their field

## Report

Since structure-lang-identity step 3 (`ObjectKey`), a clause element over a constructor field keys by one resolved copy: under a listed scope, the first listed constructor's field; under a set scope, the field's name. Two shapes give a clause a new id that main kept (both are ignored tests in evolve_ids_tests.rs):

- `type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text }`, `unique (n) in B`, `move field T.n to T.(A | B)`: the unique goes from 1025 to 1026.
- `type T { id: Key ts } = A { z: Text } | B { z: Text } | C`, `index (z) in S`, `set S of ts = A`, `rename field T.A.z to z2`: the index goes from 1025 to 1026.

catalog.md §1.2 keeps the id of every object an operator does not create or re-scope.

Direction (lead ruling, structure-lang-identity step 3b): an element keys by the field ids it reaches across its whole scope (a set scope through its current members). Every operator that replaces field ids (move down and up, split, merge, nest, unnest, and each revert) yields a map from old field ids to new ones; `ids::assign` translates the keys of the clauses the re-scope rule keeps through that map, between the field round and the index round. Dotted elements (`index (p.a)`, `unique (s.C.g)`, which lose their id on a rename on main too) resolve through the field's type the same way. Add to the searching test a one-constructor set, a move down with a clause scoped to a non-first constructor, a sum with a constructor named after its type, dotted elements, and reverts of split, merge and nest over set scopes; un-ignore the two tests.

Defect class: Catalog rows and identity by position or spelling.

## 2026-11-12: resolved
