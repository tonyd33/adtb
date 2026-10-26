---
title: "What migrations.applied holds"
pick: one
options:
  wall-clock:
    title: "The wall-clock time the migration was applied, in its transaction"
  commit-time:
    title: "The commit time of the transaction that applied it"
---
Raised while writing the migration log rows. `applied: Instant` did not
say which instant.

## spec/log-rows/applied-time/wall-clock

`applied` is the wall-clock time at which the migration was applied,
read in the transaction that applies it.

## spec/log-rows/applied-time/commit-time

The commit time. The row is written before the transaction commits, so
it cannot hold its own commit time without a second write.
