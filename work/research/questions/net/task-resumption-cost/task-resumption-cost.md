---
title: Owned tasks require resumable execution and dependency waits
pick: one
raised-by: [net/task-boundary/owned]
options:
  implement:
    title: Add resumable engine waits and bounded result production before claiming server credit support
  live:
    title: Accept blocked workers and materialized results as a temporary development limitation
---
Existing scheduler tasks are run-to-completion closures, transaction waits park
the calling thread, and public row/Arrow iterators hold completed vectors.
An owned handle wrapped around those calls alone does not deliver resumability.

## net/task-resumption-cost/implement

Fund the engine changes in provider-first slices. Demonstrate a queued commit
can run while other calls wait, result credit suspends actual production under
a bounded memory grant, cancellation removes queued/suspended work, and snapshot
resources end at the correct completion/drop/commit boundary. Use deterministic
transport-independent tests plus real threads. Keep the synchronous embedded
facade working over the same engine semantics.

## net/task-resumption-cost/live

Keep a development-only blocking adapter until execution is resumable. Report
its limitations explicitly; it cannot certify the authenticated, bounded,
credit-aware Unix server milestone. This reduces initial implementation scope
but postpones the central engine work and creates an adapter to retire.
