---
title: What a restore answers for a point it cannot reach
pick: one
options:
  one-reason:
    title: One RestorePointUnavailable with the reachable range, for out of range, a chain gap and a point on a dump
  silent-latest:
    title: Restore to the nearest reachable point
---
`restore from "b" to "n" at position 3/81234` past the backup's end has no reason in catalog.md.

## repl/restore-point/one-reason

The range tells the operator what to ask for. A dump has no point, and states it the same way.

## repl/restore-point/silent-latest

A restore to a state other than the one asked for is silent data loss.
