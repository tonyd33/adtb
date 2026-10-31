---
title: What merging or nesting a deprecated constructor does
pick: one
options:
  same-state:
    title: Sources all deprecated give a deprecated product, a mix stays refused
  refuse:
    title: Keep the refusal and make a split of a deprecated constructor lossy or refused
  no-inverse:
    title: The product starts enabled
---

The operators are closed under inverses, and `split` reverts by `merge` when its routing is invertible. Products of `split` and `unnest` take their source's state, deprecated included, while merging or nesting a deprecated constructor is refused. So a split of a deprecated constructor has an inverse that the state rules refuse. Both cannot hold.

## evolution/merge-deprecated/same-state

A merge or nest whose sources are all deprecated gives a deprecated product. A mix of deprecated with any other state stays refused. No writer gains a constructor it was stopped from, the inverse of a split or unnest of a deprecated constructor exists, and no state is invented for a mix.

## evolution/merge-deprecated/refuse

Keep the refusal, and declare a split of a deprecated constructor lossy, or refuse the split. Breaks inverse closure for a state, or removes a way to reorganise a constructor being retired.

## evolution/merge-deprecated/no-inverse

Let the product start enabled. Reopens a constructor to writers that `deprecate` stopped.
