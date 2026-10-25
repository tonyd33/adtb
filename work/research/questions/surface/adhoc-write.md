---
title: Whether the ad hoc role may write
pick: one
raised-by: [surface/adhoc/scoped]
options:
  read-only:
    title: Ad hoc is read-only; fixes are migrations or registered transactions
  audited-write:
    title: Ad hoc may write, with every change set logged
---
Production data fixes need some path.

## surface/read-only

A fix is a scripted migration step or a registered transaction.

## surface/audited-write

Writes allowed; the change set and its author are recorded.
