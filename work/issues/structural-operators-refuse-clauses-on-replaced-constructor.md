---
created-at: 2026-11-12
status: open
kind: bug
priority: 320
summary: "split, merge and nest are refused with an unrelated checker error when a default or check names a constructor they replace"
tags:
  - class-copied-rules
---

# Structural operators refuse a default or check on a replaced constructor

## Report

The structural operators rewrite uniques and indexes but not defaults or checks naming a replaced constructor:

- `default A.x = "a"` on `ts`, then `split constructor T.A into A1 {..} | A2 {..} by match ..` or `merge constructor T.(A | B) into M {..} ..`: refused with "`A` is not a constructor of `T`"; `nest constructor T.(A | B) into N { inner: Inner } ..`: "`A` is not a constructor of T" (no backticks on `T`: a second site).
- `check x_set on ts.A (scalar_len(x) > 0)`, then the same split: "`A` is not a constructor of `T`"; the nest: "the nest is refused: `ts.A` names some but not all of its constructors".

The refusals name neither the clause nor the operator. Found while writing the law search of structure-lang-identity step 5a, whose generator drops the default first to avoid it.

Direction: a ruling on what each operator does with a default or check over a replaced constructor (re-scope by the one rule, refuse with a reason naming the clause, or a `default`/`check` clause on the operator), then the law search drops its `drop default ts.A.x` setup.

Defect class: Wildcards and copied rules.
