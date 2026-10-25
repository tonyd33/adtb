---
title: Hand-written operators and the declarative file can disagree
pick: any
raised-by: [[surface/migration/handwritten, surface/source/declarative]]
options:
  check-fold:
    title: The engine folds the operators and refuses a mismatch with the file
  live:
    title: Live with the two drifting
    excludes: [surface/migration/drift/check-fold]
---
The file says `Meta`; the operators never renamed `Facebook`.

## surface/migration/drift/check-fold

`adt migration status` folds every operator from the empty schema and
compares the result with the file, structurally.

## surface/migration/drift/live

The database follows the operators; the file is documentation.
