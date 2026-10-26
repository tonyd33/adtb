---
title: "Which type a stored view's relation variable has, and its id"
pick: one
options:
  new-type:
    title: "store f creates a type for the view's element type, with one constructor, after the view's relation variable"
  result-type:
    title: "The relation's sum is the declared type the function returns; an anonymous record result gets no type and sum is optional"
---
Raised by `id-assignment-gaps`. `catalog::relations` has `sum: Key
types`, and a stored view is a relation variable of kind `Stored`. A
view usually returns `Set` of an anonymous record, which has no type
row, and §1.2's row for `store f` creates no type.

## spec/ids/view-type/new-type

`store f` creates the relation variable, then a type named after the
view with one constructor, each taking the next shared id, then the
fields of that constructor, then the indexes. `sum` always names a
row.

## spec/ids/view-type/result-type

`sum` names the declared sum the view returns, and becomes `Option
Key types`, `None` for an anonymous record. No new objects, but
readers of the catalog must handle views with no type.
