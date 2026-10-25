---
title: "Which relaxing operators are held pending for pinned readers"
pick: one
options:
  all:
    title: "`drop constraint`, `drop unique`, `drop check` and a relaxing `rekey`; pending drops are carried and rechecked, and an operator leaving one without an image is refused"
  constraint-only:
    title: "Only `drop constraint`"
---
Raised by the third review of the spec in docs/spec/.

## spec/pending-relaxations/all

`drop constraint`, `drop unique`, `drop check` and a relaxing `rekey`; pending drops are carried and rechecked, and an operator leaving one without an image is refused. Every reader sees data satisfying every constraint of its version.

## spec/pending-relaxations/constraint-only

Only `drop constraint`. A reader pinned before `drop unique` can see duplicates.
