---
created-at: 2026-10-25
status: open
kind: bug
priority: 732
summary: a key a rolled-back statement inserted makes other inserters wait and fail serialization
---

# Bug: a rolled-back statement's keys block other inserters

## Report

A statement that fails after `apply` is undone by killing the versions
it inserted with its own xid, but their index entries stay. Until the
transaction ends, another transaction inserting the same key waits on
it, then fails with a serialization error if its snapshot predates the
commit, although no row with that key ever committed. My ruling: fix
it with per-statement sub-ids, so a probe can tell a version its own
transaction later undid from a live one.
