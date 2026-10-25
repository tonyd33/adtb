---
title: "What take does with a negative count"
pick: one
options:
  fault:
    title: "Faults with NegativeCount, as top does"
  empty:
    title: "A negative count delivers no elements"
---
Raised by the reference interpreter. queries.md §14.4 and the `present`
equation of semantics.md §5.9 did not say what `take n` does when `n`
is negative.

```adt fragment
query page (n: I64) -> Set User =
  from u in users select u order by u.id take n
```

## spec/take-negative/fault

`take n` with `n < 0` faults with `NegativeCount`, the fault `top`
already raises for a negative count. One rule for both counts.

## spec/take-negative/empty

`take n` with `n < 0` is `take 0`. Total, but a caller's arithmetic
error on the count passes silently as an empty page, and `top` and
`take` disagree.
