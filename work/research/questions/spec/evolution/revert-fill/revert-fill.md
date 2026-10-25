---
title: "What a revert's fill clause is"
pick: one
options:
  per-field:
    title: "One fill per field, fill T.C.f = e, with the form and scope of add field's fill"
  per-migration:
    title: "One fill for the whole revert"
---
Raised while applying the ruling on revert identity. A revert whose
inverse adds back a dropped field takes the field's values from a
`fill` clause, and the spec did not say its form or what `e` may read.

## spec/evolution/revert-fill/per-field

Each inverse that adds a field takes its own clause, written with the
field's current name, and `e` is read as `add field`'s fill: it gives
the value for every existing row and may read the row being filled and
nothing else.

```adt
revert 0015
  fill Identity.EmailPass.hash = b""
```

## spec/evolution/revert-fill/per-migration

One fill expression covers the revert. It cannot give different
fields different values, and its type has no single field to check
against.
