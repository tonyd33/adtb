---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 408
summary: the pager has no log record for the safe-snapshot marker SSI needs
closed: 2026-10-25
tags:
  - engine
---

# Feature: Safe-snapshot marker record

## Report

`docs/design/mvcc.md` §8.3 has the primary log a marker when it finds a
safe snapshot. The pager has no such record. Add a record carrying the
snapshot (xmin, xmax, the in-progress xids) and its commit position,
and a call such as `pager.log_safe_snapshot(bytes) -> Lsn`. The
serializer already reports when a read-only snapshot is decided.

## 2026-10-25: resolved

Landed as `feat(storage): commit through a group-commit leader, log safe snapshots and size threads to the pool`.
