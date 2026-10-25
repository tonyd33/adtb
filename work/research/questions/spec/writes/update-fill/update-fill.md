---
title: "What an update's new value omits"
pick: one
options:
  value:
    title: "The new value is a value: an omitted own field takes the relation's default for its constructor; with x copies every field x has"
  keep-old:
    title: "An omitted own field keeps the old row's value when the constructor is unchanged"
---
Raised by the implementability review of docs/spec/. writes §4.1
filled omitted fields from defaults only on a retag, so the meaning of
`set Revoked { }` depended on whether the row was already `Revoked`.

```adt
update s in logins where key s == k set Revoked { }
-- at takes the relation's default whether s was Active or Revoked
```

## spec/writes/update-fill/value

In an update, the new value is a value: an own field of its constructor
`D` that the value leaves out takes the relation's default for `D`,
whether or not the row was already `D`. A field with no default must be
given; this is decided statically. `D { x with ... }` copies every
field `x` has, `D`'s own fields included when `x` is a `D`, and fills
only fields `x` lacks; a field that some constructor of `x`'s type
lacks and that has no default must be given.

## spec/writes/update-fill/keep-old

An omitted own field keeps its old value when the row stays in its
constructor. This is a partial update by omission, which writes §3.2
excludes, and the same expression means different rows by the row's
constructor at run time.
