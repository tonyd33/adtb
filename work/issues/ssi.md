---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 610
summary: "no SSI: no SIREAD locks, rw-antidependency tracking, safe snapshots or serialization failures"
closed: 2026-10-25
depends-on:
  - commit-log-snapshots
  - slotted-heap
  - blink-tree
tags:
  - engine
---

# Feature: serializable snapshot isolation

## Report

Implement `docs/design/mvcc.md` §8, §12.7 and §12.9 and
`docs/spec/transactions.md` §1 and §6: SIREAD locks on heaps and
next-key ranges, rw-antidependency tracking, deferrable read-only
transactions and safe snapshots, lock promotion, serialization
failures at snapshot edges. Settle `mvcc.md:308`, the SIREAD lock of a
hash index probe.

Prerequisites: `commit-log-snapshots.md`, `slotted-heap.md`,
`blink-tree.md`.

## 2026-10-25: resolved

Landed as `feat(mvcc): add uniqueness checks and serializable snapshot isolation`. It passes every `adtb_isolation::SCENARIOS` and a 24-seed random model under the simulator. The follow-ups are in `ssi-integration.md`.
