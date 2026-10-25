---
title: Consensus needs a replicated log beside the redo log
pick: any
raised-by: [repl/failover/raft, repl/failover/vsr]
options:
  one-log:
    title: The redo log is the consensus log; each record carries term and index
    links:
      - {rel: gains, to: buffer/group-commit/leader}
  two-logs:
    title: A consensus library keeps its own log; committed entries are then written to the redo log
  live:
    title: Live with two fsyncs per commit
    excludes: [repl/failover/wal-log/one-log, repl/failover/wal-log/two-logs]
---
Libraries like openraft own log storage through a trait; the engine
already has a redo log with group commit and aligned slots.

## repl/failover/wal-log/one-log

The group-commit leader is the proposer: one batch, one local
`fdatasync`, one append to followers. Uncommitted tail records may be
truncated by a new leader. Their transactions never committed, so
their row versions stay invisible, but a deposed leader's pages may
already carry page-level effects (splits, new versions) of the
truncated records; it must resync those pages from the new leader, as
`pg_rewind` does (unverified), since there is no undo.

## repl/failover/wal-log/two-logs

Simple to build on a library; each commit is forced twice.

## repl/failover/wal-log/live

Accept the second force.
