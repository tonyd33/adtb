---
title: Updating one element of a nested value rewrites the value
pick: any
raised-by: [model/nf2, model/scalar-collections, model/recursive-values]
options:
  path-update:
    title: Updates may address a path inside a nested value and storage patches it
    links:
      - {rel: variant-of, to: write/nested/path}
  immutable:
    title: Nested values are written whole at insert and never updated
  live:
    title: Live with updates that replace the field's whole value
    excludes: [model/nested-update/path-update, model/nested-update/immutable]
---
Changing one line's `qty` in a list of a thousand lines.

## model/nested-update/path-update

```
update o in orders where o.id == 7 set lines[sku == "x"].qty = 3
```

## model/nested-update/immutable

A changing collection is a child relation; a nested value is a fact
recorded once.

## model/nested-update/live

```
update o in orders where o.id == 7
set Order { o with lines: from l in o.lines
                          select if l.sku == "x" then Line { l with qty: 3 } else l }
```
