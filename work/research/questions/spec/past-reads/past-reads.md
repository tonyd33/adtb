---
title: "Which state `at commit p` reads"
pick: one
options:
  safe-snapshot:
    title: "The greatest safe snapshot at or before p, listed in sys::safe_snapshots, typed at that snapshot's schema version; p is a Position; no SIREAD locks"
  commit-boundary:
    title: "The state just after commit p"
---
Raised by the third review of the spec in docs/spec/.

## spec/past-reads/safe-snapshot

The greatest safe snapshot at or before p, listed in sys::safe_snapshots, typed at that snapshot's schema version; p is a Position; no SIREAD locks. Under SSI an arbitrary commit boundary may be a state no serial order produces.

## spec/past-reads/commit-boundary

The state just after commit p. Can expose the read-only anomaly without SIREAD locks.
