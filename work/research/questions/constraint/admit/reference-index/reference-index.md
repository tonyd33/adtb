---
title: How a reference field's check stays index-bounded
pick: one
raised-by: [constraint/admit/bounded, model/typed-key]
options:
  implied:
    title: Each reference field has an implied index on the referring field over the referring constructor set
  declared:
    title: A reference is refused unless a declared index leads with the referring field
  exempt:
    title: References are checked by a dedicated mechanism outside the bound
---
Every constraint is index-bounded, references included, yet a
reference field needs no declared index, and a check on a deleted or
moved target has to find its referrers.

```adt fragment
type Identity = { id: Int, user: Key users }
relation identities : Identity key id
-- deleting a user must find the identities whose user is its key
```

## constraint/admit/reference-index/implied

The engine builds and maintains an index on the referring field over
the referring constructor set, like a declared index, unless a
declared index already has that field as its leading field. The
catalog lists it as implied by its reference. Target checks and delete
actions probe it.

## constraint/admit/reference-index/declared

A reference field without a declared index leading with it is refused
at declaration, naming the missing index. Every schema with a
reference spells the index out.

## constraint/admit/reference-index/exempt

References are checked by scanning the referrers on a target's delete
or move. The bound holds for every constraint except references.
