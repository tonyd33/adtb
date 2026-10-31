---
created-at: 2026-10-30
status: open
kind: bug
priority: 468
summary: "an open that cannot free a crashed schema change's pages leaves changing set, so every reindex fails with Internal and the damage can never be repaired"
tags:
  - quality
  - engine
---

# Catalog open damage blocks reindex

## Report

`Catalog::open` at `crates/adtb/src/catalog/mod.rs:691` ignores damage from `free_leaked` and goes on, leaving `changing` on the root. Its doc at line 678 still says it fails with the damage found.

With `Changing::Schema(x)` or `NamingSequences` left set, `begin_reindexing` at line 1004 refuses with `Internal("a rebuild of indexes began during the catalog's change ...")`, and `rebuild_all` (`crates/adtb/src/api/reindex.rs`) records that as the finding of every index. So the two wait on each other: the open frees the crashed change's pages only once the damage is gone, and `reindex` restores a damaged index only once `changing` is clear.

Found by reading; not run. Repro: crash a migration before its commit, damage a page of a user index, reopen, run `reindex` on it. Expect the index restored; reading gives the `Internal` finding.

Fix direction: let a rebuild start over a leftover change that no running transaction owns, or finish the free at the next open once the rebuild has repaired the damage. Correct the doc either way.
