---
title: SSI commit order is not a serial order, so replaying inputs diverges
pick: any
raised-by: [repl/unit/command]
options:
  serial-order:
    title: The primary logs a serial order derived from its dependency graph, not commit order
  deterministic-cc:
    title: Replace SSI with sequencer-first deterministic execution
    excludes: [engine/ssi]
  live:
    title: Live with replicas that may compute different results
    excludes: [repl/unit/commit-order/serial-order, repl/unit/commit-order/deterministic-cc]
---
T1 reads `User` 3, T2 updates `User` 3 and commits, T1 commits. SSI
accepts this with T1 serialised before T2, yet commit order puts T2
first; re-running T1 after T2 reads the new row and may write
something else. Ports and Grittner make the same observation for
read-only replica transactions.

## repl/unit/commit-order/serial-order

Each commit record carries a position in a topological order of the
rw-dependency graph, and replicas execute in that order, serially or
with deterministic locking. The engine must compute an order SSI only
proves exists (unverified that this is cheap).

## repl/unit/commit-order/deterministic-cc

Calvin: order first, then execute. Contradicts the accepted SSI.

## repl/unit/commit-order/live

Replicas diverge.
