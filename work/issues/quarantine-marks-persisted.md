---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 480
summary: quarantine marks are not persisted; a quarantined heap or index is found again only by recovery or verify
closed: 2026-10-25
depends-on:
  - pager-and-log
tags:
  - engine
---

# Bug: Quarantine marks are not persisted

## Report

A page that fails verification quarantines its heap or index
(`docs/design/storage.md` §8), but the mark lives only in memory. After
a restart the heap or index is open again until recovery or `verify`
reads the damaged page. Expected: the mark survives a restart, and a
quarantined heap or index stays refused until it is repaired.

## 2026-10-25: resolved

Landed as `feat(storage): persist quarantine marks and read the log a chunk at a time`.
