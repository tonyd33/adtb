---
created-at: 2026-11-12
status: open
kind: bug
priority: 310
summary: "a split whose carry renames a field rewrites an index over a one-constructor set to the first product, dropping the set name and the id"
tags:
  - class-identity-by-spelling
---

# A split with a renamed carry drops a set-scoped index

## Report

`set S of ts = A`, `index (y) in S`, then a split `A a -> A1 { x: a.y, y: a.x }`: the clause becomes `index (x) in A1` with id 1027, though it names no replaced constructor. With two products, S becomes `A1 | A2` and A2's rows are no longer covered; the revert gives `index (y) in A`, id 1029. Pre-existing; the one-constructor branch of the split's clause rewrite (evolve/ctor/split.rs) splits per product whenever a field is carried under another name. Under the keying of structure-lang-identity step 3b, `index (x) in S` would keep the id (`A.y` maps to `A1.x`; `A2.x` is created without replacing anything), so the exception is the text rewrite.

Direction: a clause naming no replaced constructor keeps its scope; its elements are rewritten through the carry where every product keeps the field (under one name), else the split is refused naming the clause, as for several-constructor scopes. Test: the scenario with one and with two products, ids and texts against catalog.md §1.2.

Defect class: Catalog rows and identity by position or spelling.
