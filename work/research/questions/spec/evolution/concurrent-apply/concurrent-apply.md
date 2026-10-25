---
title: "What applying a migration does while a staged migration runs"
pick: one
options:
  refuse:
    title: "Refused with MigrationInProgress"
  wait:
    title: "Waits for the staged migration to finish"
---
Raised by the implementability review of the spec in docs/spec/. The
spec said no other migration applies while one is pending, but not
what a second apply observes.

## spec/evolution/concurrent-apply/refuse

Applying another migration while a staged migration runs is refused
with `Rejected { reason: MigrationInProgress }`. The deploy tool
retries after the job ends.

## spec/evolution/concurrent-apply/wait

The second apply waits. A staged job can run for hours, so the deploy
hangs with no report, and the waiting set is checked against a version
it did not see when it started.
