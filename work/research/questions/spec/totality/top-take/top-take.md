---
title: "Whether top and take are partial"
pick: one
options:
  non-literal:
    title: "Partial unless the count is an integer literal"
  always:
    title: "Always partial"
---
Raised while writing the reference interpreter. `top` and `take` fault
with `NegativeCount` on a negative count (queries.md §14.3, §14.4), but the
list of partial expressions in typing.md §10.1 names prelude functions
and other forms, not these two.

## spec/totality/top-take/non-literal

A `top` or `take` is partial when its count is not an `INT` literal. A
literal has no sign (syntax.md §2.5), so it is never negative, and
`top 3 by ...` stays total:

```adt fragment
top 3 by p.created_at desc (from p in posts select p)   -- total
-- given n: I64
top n by p.created_at desc (from p in posts select p)   -- partial
```

## spec/totality/top-take/always

Every `top` and `take` is partial. One rule, but a constant page size
makes a query partial for no reason, and partial expressions are held
under their guards.
