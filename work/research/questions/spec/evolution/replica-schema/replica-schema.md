---
title: "What a replica guarantees about schema changes"
pick: one
options:
  safe-snapshot:
    title: "Replicas serve consistent safe snapshots, so no row of a constructor appears before its declaration"
  apply-order:
    title: "The spec states the order in which a replica applies schema records"
---
Raised by the implementability review of the spec in docs/spec/.

## spec/evolution/replica-schema/safe-snapshot

Replicas serve consistent safe snapshots, so a replica never shows a
row of a constructor before the constructor's declaration, and a
snapshot's schema version and data are those of one point of the
primary's history.

## spec/evolution/replica-schema/apply-order

The spec states that a replica applies a declaration before any row of
it and a stage transition before any change that depends on it. A
procedure, and it does not by itself give a reader a consistent view.
