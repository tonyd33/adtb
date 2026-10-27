---
title: "How an operator gives a reference field its delete action"
pick: one
raised-by: [evolution/smo]
options:
  field-type-and-alter:
    title: "add field takes a field type, on delete included; alter field changes the action of an existing reference field"
  alter-only:
    title: "add field adds a restrict field; only alter field changes the action"
  fixed:
    title: "The action is fixed when the field is declared"
---
Raised by the campaign manager. `add field` takes a `type`, and a delete
action belongs to a field type, so no operator gives a new or existing
reference field an `on delete` action.

## spec/evolution/field-action/field-type-and-alter

`add field T.C.f: U on delete clear fill e` declares the action with
the field. `alter field T.C.f on delete cascade` changes the action of
an existing reference field; its inverse restores the old action.

## spec/evolution/field-action/alter-only

`add field` keeps taking a type and adds the field `restrict`; a
following `alter field` gives it another action.

## spec/evolution/field-action/fixed

Delete actions are written only in `add type` and `add constructor`. A
field that needs another action is dropped and added again, losing its
values.
