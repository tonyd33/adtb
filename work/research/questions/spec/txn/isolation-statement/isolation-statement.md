---
title: "How the isolation guarantee is stated"
pick: one
options:
  serial-order:
    title: "As the serial-order property of the committed transactions alone"
  tracked-predicates:
    title: "As an acyclic graph over the predicates the implementation tracks"
---
Raised by the tier review of docs/spec/. semantics §8.2 defined reads
through "predicates the implementation tracks", which makes the
guarantee relative to an implementation.

## spec/txn/isolation-statement/serial-order

The guarantee is that the committed transactions have the outcomes and
final state of some serial order of them. How reads are tracked is
implementation.

## spec/txn/isolation-statement/tracked-predicates

The guarantee names the tracked predicates. Two implementations
tracking differently promise different things.
