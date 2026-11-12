---
created-at: 2026-11-12
status: open
kind: bug
priority: 440
summary: "split, merge, nest and their reverts rewrite uniques so they cover other rows than before, weakening or strengthening the declared constraint"
tags:
  - class-copied-rules
---

# Structural operators change the rows a unique covers

## Report

The search `a_structural_operator_or_its_revert_takes_the_rows_a_clause_covers_to_those_of_the_clause_after` in crates/adtb-lang/src/evolve_ids_tests.rs (ignored, added by structure-lang-identity step 3b) checks that a kept clause covers the image of its old rows under the operator's field map, and that a re-scoped clause is replaced by one covering exactly that image. It fails in four classes, all pre-existing in the clause rewriting:

- split: `unique (z) in A` becomes `unique (z) in A1` and `unique (z) in A2`, so z is no longer unique across the rows A held (weaker); its revert gives `unique (q, z) in A` (split-revert-adds-discriminator). Same for `unique (s.X.g)`.
- merge: `unique (z) in A` becomes `unique (k, z) in M`, which also constrains rows from B (stronger for B); `unique (n) in B` likewise. evolution.md (the operator effects table, merge row) states this rewrite: "per-constructor uniques become uniques including the discriminator". A joint unique over the merged constructors is weakened (merge-weakens-joint-unique).
- nest: `unique (n) in B` over header field `n` becomes `unique (n) in N`, reaching rows from A (stronger); its revert gives `unique (n) in (A | B)` (nest-revert-widens-unique).
- remove a constructor, reverted: set `S` stays `A | B`, so clauses over `S` no longer cover the restored `C`.

A rewritten unique that covers more rows can refuse a migration's own data or later writes the original schema accepted; one that covers fewer lets duplicates in.

Direction: a design ruling first. Either the rewrite keeps exact coverage (a split keeps one unique over the products, `in (A1 | A2)`; a merge's per-constructor unique needs a form scoped to one discriminator value, or the merge is refused while it holds one), or the spec states each change of coverage and the migration validates it against the data. Then un-ignore the search. Folds split-revert-adds-discriminator, nest-revert-widens-unique and merge-weakens-joint-unique if the ruling covers them.

Defect class: Wildcards and copied rules.

## 2026-11-12

Since structure-lang-identity step 3b a merge refuses while a unique's scope (listed, full or a set) is two or more of exactly the merged constructors: "`unique (z) in (A | B)` holds across the merged constructors; the merge would weaken it". A unique that also reaches a constructor outside the merge is re-scoped without the discriminator (`(A | B | C)` to `(M | C)`, same fields). A split's revert, a derived merge, is refused the same way when the split held such a unique (`S = A`, `unique (z) in S`, split A into A1 | A2, revert). docs/spec/schema-change.md's merge row ("a unique over sources becomes a unique over `M` on the discriminator") now states more than the code does; reconcile it in the ruling.
