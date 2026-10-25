---
title: How sum values compare and sort
pick: one
raised-by: [sum/value]
options:
  derived:
    title: Declaration order of constructors, then fields
  eq-only:
    title: Equality only; ordering needs an explicit key
  declared:
    title: Each sum declares whether and how it is ordered
---
`order by a.status`, `group a by a.status`, a range index or `min` over a
sum-typed column all need an order on sum values. Equality is
constructor then fields.

## lang/sum-order/derived

```
type Status = Draft | Sent { at: Time } | Paid { at: Time, ref: Text }
-- Draft < Sent { ... } < Paid { ... }; within a constructor, fields left to right
```

Haskell `deriving Ord`. Reordering constructors in the schema changes
query results, so it becomes an evolution operator.

## lang/sum-order/eq-only

```
order by match a.status | Draft -> 0 | Sent s -> 1 | Paid p -> 2
```

Rust enums without `PartialOrd`. Sorting by a sum is a type error.

## lang/sum-order/declared

```
type Status ordered = Draft | Sent { at: Time } | Paid { at: Time, ref: Text }
```

Opt-in derived order; unordered sums give eq-only.
