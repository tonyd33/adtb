---
created-at: 2026-10-28
status: open
kind: bug
priority: 830
summary: "recursive fix equations take their row origin only from the pre-pass, the pre-pass repeats synth side effects, and constructor restriction is untested on a multi-constructor sum"
tags:
  - lang
  - quality
---

# Row provenance follow-ups

## Report

Found by the independent review of the fix-row-provenance repair (work/briefs/review-fix-row-provenance.md, local).

- crates/adtb-lang/src/check/expr.rs, the recursive binder check: `let o = self.check(e, &want)?` discards `o.rows`, so the rows of a recursive equation come only from the pre-pass in check/rows.rs. Sound today; an overclaiming pre-pass rule would go unnoticed. Refuse, or iterate to a joint fixpoint, when the checked body's origin is not below the installed one. A plain refusal rejects the widened select `fix rows = shapes union (from r in shapes.Circle select r)`.
- crates/adtb-lang/src/check/rows.rs: speculative `synth` calls repeat side effects, and reference recording is duplicated. Check whether a nested `key (insert ...)` is hoisted, and so run, twice.
- Constructor restriction is tested only as `rows.Row` on a one-constructor record. Add a case on a sum with several constructors.
- A nested fix is solved again at every enclosing iteration, so checking cost grows with nesting depth. Not measured.
- Conservative precision gaps that give unknown origin: field, backlink, `at`, record-payload binders, grouped comprehensions and widened selects.
