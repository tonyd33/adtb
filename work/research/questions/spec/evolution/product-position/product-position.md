---
title: "Where merge, nest and unnest products and re-added constructors sit in declaration order"
pick: one
options:
  first-source:
    title: "A merge or nest product takes its first source's position; unnest products take the source's; a re-added constructor its position before removal"
  append:
    title: "Every such constructor goes last"
---
Raised while applying the ruling on where an added constructor sits.
Declaration order is the order of an ordered sum, and the spec placed
split products and `add constructor` but not the products of merge,
nest and unnest, nor a constructor that an inverse re-adds.

## spec/evolution/product-position/first-source

The product of a merge or nest takes the position of its first source
in declaration order. The products of an unnest take the source's
position, in the order of the field's sum. A constructor re-added by
the inverse of `remove` takes the position it had before the removal,
so an inverse restores the prior state:

```adt
-- type Stage ordered = Draft | Sent { at: Instant } | Paid { at: Instant, ref: Text }
remove constructor Stage.Sent
-- revert of it: Draft < Sent < Paid again
```

## spec/evolution/product-position/append

Each such constructor goes last. A revert of a `remove` then moves the
constructor to the end, which changes the order of an ordered sum, so
an inverse no longer restores the prior state.
