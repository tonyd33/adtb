---
title: Whether a reference may sit inside a nested value
pick: one
raised-by: [[sum/value, fk/to-sum]]
options:
  refs-anywhere:
    title: A reference may appear at any depth of a field's value
  refs-top-level:
    title: References are only direct fields of a relation's constructors
---
`owner: User { user: Key users } | Team { team: Key teams }` is a value-level sum of
references: an EER category, which `sum/level.other` was defeated by
reducing to relation-level sums of references.

## model/refs-anywhere

```
type Doc = { owner: Owned { user: Key users } | Shared { team: Key teams }, tags: List (Key tags) }
```

The engine extracts every reference path into its FK index, so a delete
of a team finds `docs` rows whose `owner` is `Shared` it.

## model/refs-top-level

A sum of references becomes a relation-level sum:

```
type Ownership { doc: Key docs } = Owned  { user: Key users }
                                  | Shared { team: Key teams }
relation ownerships : Ownership key doc
```

Every FK is a field of a constructor relation and indexed as such.
