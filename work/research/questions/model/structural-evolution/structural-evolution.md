---
title: Structural types have no name for schema operators to act on
pick: any
raised-by: [model/structural]
options:
  aliases:
    title: Declared names are aliases that operators track
  live:
    title: Live with evolution by shape
    excludes: [model/structural-evolution/aliases]
---
`evolution/rename` and `evolution/add` act on a named sum. Under
structural typing adding a constructor to `Identity` makes a new type,
and every other use of the old shape is a different type that did not
change.

## model/structural-evolution/aliases

`type Identity = ...` names a shape; operators target the name and
rewrite the uses that mention it. Uses spelled out by shape stay as
they were.

## model/structural-evolution/live

An operator changes a relation's element type and whatever was
structurally equal no longer is.
