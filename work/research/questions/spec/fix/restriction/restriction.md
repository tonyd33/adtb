---
title: "Whether a restriction e ▷ S is a positive position"
pick: one
options:
  positive:
    title: "Positive: if e ▷ S is positive, so is e"
  not-positive:
    title: "Not positive"
---
Raised by the stratification analysis. The list of positive positions
in semantics.md §5.10 did not mention restriction, so `reach.Open`
inside a `fix` body fell under "every other position".

```adt fragment
fix reach = (from s in start select s)
          union (from r in reach.Open, n in next where n.src == key r select n)
in reach
```

## spec/fix/restriction/positive

Restriction keeps a subset of its operand, so it is monotone, and a
`fix` binder under it stays positive.

## spec/fix/restriction/not-positive

Refuses every recursive query that reads one constructor of its own
relation, though restriction is monotone.
