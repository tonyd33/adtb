---
title: "What a staged migration means to observers"
pick: one
options:
  one-transaction:
    title: "One transaction serialized at its final commit, plus the progress row and the failure report"
  intermediate-states:
    title: "A sequence of committed intermediate states, each visible"
---
Raised by the implementability review of the spec in docs/spec/. The
spec described a staged change by its physical intermediate states and
their commits.

## spec/evolution/staged-meaning/one-transaction

A staged migration is equivalent to one transaction serialized at its
final commit, plus the observable progress row and failure report. The
intermediate states are implementation.

## spec/evolution/staged-meaning/intermediate-states

The spec states each intermediate state and its commit. That fixes one
build strategy for every engine and exposes states no query can name.
