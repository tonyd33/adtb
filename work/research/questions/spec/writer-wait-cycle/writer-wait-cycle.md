---
title: "What happens when writers wait on each other in a cycle"
pick: one
options:
  fail-one:
    title: "One transaction of the cycle fails with a serialization failure; which one is the engine's choice"
  timeout:
    title: "Each waits until a lock timeout"
---
Raised by `spec-writer-wait-cycle`. `transactions.md` §6.3 states that a
writer waits for another's uncommitted write, and nothing about two
writers that each wait for the other.

## spec/writer-wait-cycle/fail-one

A cycle of waits fails one of its transactions with a serialization
failure, which §6.1 allows any transaction to receive; which one is a
freedom of the engine. The others proceed.

## spec/writer-wait-cycle/timeout

The waits last until a timeout. Clients see a stall before the
failure.
