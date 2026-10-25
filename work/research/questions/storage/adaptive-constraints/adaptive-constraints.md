---
title: Constraint machinery must survive layout changes
pick: any
raised-by: [storage/adaptive]
options:
  logical-rowid:
    title: Layout-independent logical row ids under every index
  pragma:
    title: Layout chosen by an explicit per-sum declaration
  live:
    title: Live with rebuilding indexes on each layout switch
    excludes: [storage/adaptive-constraints/logical-rowid, storage/adaptive-constraints/pragma]
---
Indexes holding physical TIDs break when a sum moves between layouts;
a switch is a rewrite. Automatic switching makes performance hard to
predict or test.

## storage/adaptive-constraints/logical-rowid

Indexes point at a logical id resolved through a mapping; layout
changes rewrite only the mapping.

## storage/adaptive-constraints/pragma

```
relation identities : Identity
  key id
  storage columnar
```

Changing it is an explicit, offline rewrite.

## storage/adaptive-constraints/live

Rebuild in the background during the switch.
