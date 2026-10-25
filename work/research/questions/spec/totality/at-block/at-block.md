---
title: "Whether an at block is total"
pick: one
options:
  partial:
    title: "Partial, because HistoryUnavailable is an abort"
  total:
    title: "Total, with HistoryUnavailable outside totality like a limit"
---
Raised by the totality analysis. An `at` block is refused at run time
with `HistoryUnavailable` when its position is outside the retention
window (derived.md §6.2). semantics.md §3.4 did not list it.

## spec/totality/at-block/partial

An `at` block is partial. It keeps its place under its guards, and an
implementation evaluates it only where the program does.

## spec/totality/at-block/total

The rejection is treated like a limit, outside totality. An
implementation could then hoist an `at` block out of a guard that
tests the position, and reject a statement whose program never reads
that position.
