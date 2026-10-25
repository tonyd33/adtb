---
title: "What deleting a target does to an optional reference"
pick: one
options:
  clear:
    title: "on delete clear sets an Option (Key) field to None"
  restrict-only:
    title: "Optional references only restrict"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/option-ref-delete/clear

on delete clear sets an Option (Key) field to None. The common case needed a hand-written update in every deleting statement.

## spec/option-ref-delete/restrict-only

Optional references only restrict. Forces manual updates.
