---
title: "What a failed staged job does"
pick: one
options:
  discard:
    title: "The pending migration is discarded and the log is unchanged"
  auto-revert:
    title: "The engine appends the inverse migration"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/staged-failure/discard

The pending migration is discarded and the log is unchanged. A version commits only when its staged jobs complete.

## spec/staged-failure/auto-revert

The engine appends the inverse migration. Undefined when the migration has no inverse.
