---
title: Whether the current schema is printed for review
pick: one
raised-by: [surface/source/op-log]
options:
  generated:
    title: The engine prints the folded schema; the printout is committed and diffed in review
  none:
    title: The log is the only text; the catalog is queried for the rest
---
With the operator log as the source, no file shows the schema as it
stands.

## surface/generated

`adt schema print > schema.adt`; CI checks the committed printout
matches the fold of the log, so a review sees both the operator and its
effect.

## surface/none

Catalog queries only.
