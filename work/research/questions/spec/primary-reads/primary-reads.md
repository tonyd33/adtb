---
title: "Whether reads on a primary wait for a safe snapshot"
pick: one
options:
  never:
    title: "Never; they run under SSI; positions carry the epoch"
  wait:
    title: "Every single-call read waits"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/primary-reads/never

Never; they run under SSI; positions carry the epoch. Reads must not wait for writes; read-your-writes must survive failover.

## spec/primary-reads/wait

Every single-call read waits. Contradicts non-blocking reads.
