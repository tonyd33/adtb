---
created-at: 2026-10-30
status: in-progress
branch: reindex-jobs
kind: feature
priority: 412
summary: "a quarantined index or heap has no user path back: reindex and restore must call pager.release"
---

# Feature: Quarantine release

## Request

No path reaches users: `pager.release` has no caller outside tests. Marks from quarantine-wired stay until reindex-jobs releases an index, and a heap needs a restore. Order: reindex first, since a false or real index mark refuses writes for good. Decide audit R4 with the backup slice.

## Proposal

## Open questions
