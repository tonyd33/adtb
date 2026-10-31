---
created-at: 2026-10-30
status: open
kind: feature
priority: 720
summary: "a quarantined constructor relation has no user path back: restore must release its mark"
---

# Feature: Quarantine release

## Request

No path reaches users: `pager.release` has no caller outside tests. Marks from quarantine-wired stay until reindex-jobs releases an index, and a heap needs a restore. Order: reindex first, since a false or real index mark refuses writes for good. Decide audit R4 with the backup slice.

## Proposal

## Open questions

## 2026-10-30

Indexes done: the reindex swap clears the mark. A heap waits for dump-restore-backup, which must call the release in its restore.
