---
created-at: 2026-11-06
status: open
kind: bug
priority: 140
summary: "a commit-log status page that lost a write settles an id aborted when the log proving its status is recycled, silently dropping an acknowledged commit"
tags:
  - class-damage-unreported
---

# settling a status the recycled log cannot prove

## Report

When a commit-log status page loses a write and is settled from the log, an id whose commit record has been recycled reads as aborted (mvcc.md §4.2), the status page standing as authoritative: an acknowledged commit silently dropped. Today it reaches commits of sessions before the last; with in-session recycling (storage/readin-gate/recycle as amended 2026-11-06) it would reach any commit older than the floor. Ruled 2026-11-06 (mvcc/settle-recycled: refuse): a status the retained log cannot prove is recorded as damage and quarantines the database, never settled aborted. Must land before periodic-checkpoint change 2.
