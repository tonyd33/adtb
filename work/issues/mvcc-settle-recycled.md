---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 140
summary: "a commit-log status page that lost a write settles an id aborted when the log proving its status is recycled, silently dropping an acknowledged commit"
closed: 2026-11-07
tags:
  - class-damage-unreported
---

# settling a status the recycled log cannot prove

## Report

When a commit-log status page loses a write and is settled from the log, an id whose commit record has been recycled reads as aborted (mvcc.md §4.2), the status page standing as authoritative: an acknowledged commit silently dropped. Today it reaches commits of sessions before the last; with in-session recycling (storage/readin-gate/recycle as amended 2026-11-06) it would reach any commit older than the floor. Ruled 2026-11-06 (mvcc/settle-recycled: refuse): a status the retained log cannot prove is recorded as damage and quarantines the database, never settled aborted. Must land before periodic-checkpoint change 2.

## 2026-11-07: fixed

`fix: record a stale status page the retained log cannot prove as damage instead of settling it aborted`. One gate, `CommitLog::unresolved`, through which every settle, the quarantined open and dump admission pass: a stale copy holding an id in progress is refused (its Stale damage recorded, the database quarantined as for commit-log damage) unless the retained log holds a record of the page continuing from the copy. A status page stores its page number at allocation (format 2), and a copy that is not status page n is refused: settle is the one reader of a copy the read-in gate refused, and the number is that copy's identity (page-incarnation-identity, M2). Cause: settle read no commit record in the retained log as aborted even when the log no longer reached the copy, and trusted any copy at the pid. Class class-damage-unreported; the other page kinds' reuse goes to page-incarnation-identity's M1. Tests in clog_lost: a recycled commit (with and without an archive) and a recycled abort quarantine, a retained commit settles committed, and a status page reverted to an older incarnation at its pid is refused while intact pages' ids stay in progress; the recycled cases and the reverted page read the commit as aborted on main. Review: one blocker (the reused pid), fixed; delta review clean. Left: the txn.rs open postcondition does not name the not-this-page refusal; `resolutions_from`'s unit tests check only commits.

## 2026-11-07: resolved
