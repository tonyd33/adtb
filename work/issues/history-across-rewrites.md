---
created-at: 2026-10-27
status: open
kind: feature
priority: 873
summary: "a migration that rewrites a relation, and remove constructor, free the old heaps at once, so history across them is lost"
depends-on:
  - history-catalog
tags:
  - engine
---

# Feature: keep history across a rewriting migration

## Report

`docs/design/mvcc.md` section 10.4 keeps a removed constructor's heap until the window no longer reaches it, and `drop_empty` with its crash tests exists, but migrations rewrite the relation and free the old heaps at the migration, so `remove constructor` and every rewriting operator lose history. Past reads are as written (ruling `derived/time-travel/past-schema/as-written`), so a rewritten relation's old heaps must stay readable at their version until the window passes them. Decide in the design file how a rewrite hands its old heaps to history, then make `remove constructor` keep its heap.

## 2026-10-29: contradictory HistoryUnavailable

Found by the manual-gaps track. After a migration that rebuilt the relation, `HistoryUnavailable`
names an oldest readable position that is itself refused when read, and a read past the last
commit names an older position. The error's position must be readable.

A `rename field Tool.Borrowed.days to loan_days` also makes every `at commit` read of the relation
before it answer `HistoryUnavailable` (manual reader check, 2026-10-29). A rename changes no
stored value, so it should not end history.

Manual round 2 probe (2026-10-29), each operator after two writes: `add field`, `drop field`,
`rename field`, `add constructor`, `add index` and `add unique` end a relation's history; `add
check`, `add constraint`, `add default`, `define fn`, `rename constructor` and adding another
relation keep it. Of these only a rebuild of stored values should end history.
