---
created-at: 2026-11-15
status: open
kind: bug
priority: 650
summary: "write::delta and plan::lower keep four memoised recursive rewrites that repeat Terms::substitute, and they differ on whether a mapped term is memoised"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Four engine term rewrites repeat Terms::substitute by hand

## Report

Found by the quality review of reviewed..582d7af5. The range added `Terms::substitute` (crates/adtb-lang/src/kernel.rs:454), a memoised, stack-based rewrite that rebuilds each term above a replaced one through `Build::rebuild_kept`. Four engine rewrites still do the same by hand, each recursive and each with its own memo:

- `Compiler::replace`, crates/adtb/src/write/delta/mod.rs:1067 (map of terms to images);
- `Compiler::subst`, crates/adtb/src/write/delta/mod.rs:1086 (variable renaming);
- `replace`, crates/adtb/src/plan/lower/group.rs:253 (map of terms to images, memoising a mapped term, which the delta copy does not);
- `rename`, crates/adtb/src/plan/lower/split.rs:193 (one variable renamed).

Each is `children, map, if new == cs { t } else { rebuild_kept }`, the body of `substitute` with an image closure. Being recursive, each also recurses over kernel depth, which `substitute` avoids (kernel-depth-audit audits recursion, not this duplication). `delta::subst`'s doc says it renames free variables, but it renames every occurrence, bound or not, as `substitute` with a `Var` image would; both rely on variables being unique per arena.

Fix direction: express each as `terms.substitute(schema, t, |terms, at, _| ...)` and delete the copies; state in `substitute`'s doc that an image renames under binders.
