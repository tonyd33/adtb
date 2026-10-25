---
title: "What the spec states about index structures"
pick: one
options:
  declared-capabilities:
    title: "The index forms a schema declares and what each lets a query do; not the structure behind them"
  ordered-trees:
    title: "Every index is an ordered tree"
---
Raised by the tier audit. interop §11.2 said every index is an ordered
tree, while the design allows a hash index for an equality unique
(ruling `btree/hash/eq-unique`).

## spec/index/kinds/declared-capabilities

The spec states each declared index form and what it lets a query do
(equality, range, order). Whether an equality unique uses a hash
index is design.

```adt
index (created_at) in Post     -- range and order on created_at
unique (email) in EmailPass    -- equality on email
```

## spec/index/kinds/ordered-trees

Promise ordered trees for every index. Rules out the hash index and
ties the spec to one structure.
