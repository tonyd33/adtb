---
created-at: 2026-11-14
status: open
kind: bug
priority: 300
summary: "the constructor operators branch on a clause's scope as written, not as the checker reads it"
tags:
  - lang
  - class-identity-by-spelling
---

# the constructor operators branch on a clause's scope as written, not as the checker reads it

## Report

Found in the review of structure-lang-identity step 8a (a1436ec3), which reads a scope naming every constructor as none at the term level.

constructor operators branch on a clause's scope as written, not as the checker reads it".
`evolve/ctor/merge.rs:143` (`merged_clauses`) and the split and nest paths of
`evolve/ctor/split.rs` read `cl.scope` as the source text spells it, so `unique (n) in (A | B)`
listing every constructor of `T` refuses a merge of `A | B` ("holds across the merged
constructors; the merge would weaken it") that plain `unique (n)`, the same clause to the
checker, allows. Repro: `type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text }`,
`relation ts : T key id generate sequence unique (n) in (A | B)`, then
`merge constructor T.(A | B) into M { k: K, z: Text } by match | A a -> M { k: A, z: a.z } | B b -> M { k: B, z: b.z }`
with `type K = A | B`. The fix reads the scope normalised (`Schema::normalize_ctors`).

Defect class: Catalog rows and identity by position or spelling.
