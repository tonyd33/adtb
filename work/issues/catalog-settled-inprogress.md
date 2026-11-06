---
created-at: 2026-11-05
status: open
kind: bug
priority: 420
summary: "under a whole-database mark the catalog reads a readable InProgress status as settled-uncommitted, though the retained log may prove it committed"
tags:
  - class-damage-unreported
---

# Catalog trusts InProgress as uncommitted under a commit-log quarantine

## Report

## 2026-11-05

Found by the clog-lost scoped review. `latest` and `settled` (crates/adtb/src/catalog/mod.rs:465-466, crates/adtb/src/mvcc/txn.rs:173-178) treat a readable `InProgress` status as settled-uncommitted. That includes ids the readable part of the retained log proves committed: `logged` holds them, but the `Ok(InProgress)` status wins. After an open that settled nothing (the log-unproven quarantine), in progress is exactly the unknown state, so a committed migration can read as not applied.

storage.md §8: the open trusts a catalog row only when its writer's and deleter's status is read from an undamaged status page, or a retained commit record proves it committed; a row of an id neither settles leaves the whole catalog unread.

Direction: `settled` returns `None` for `InProgress` while the database is under a commit-log mark, and a commit in `logged` takes precedence over an `InProgress` status. Test: a committed migration whose status write was lost, under the mark; verify reports the catalog unread instead of the old schema.

Defect class: Damage reported as nothing.
