---
created-at: 2026-11-10
status: open
kind: bug
priority: 800
summary: "the commit log's pages are listed by hand at three sites in two orders: catalog structures, recorded_damage and Catalog::load"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Bug: the commit log's page list is copied three times

## Report

`commit_log_pages` (crates/adtb/src/catalog/structures.rs:114), `recorded_damage` (crates/adtb/src/mvcc/txn.rs:84-88) and `Catalog::load` (crates/adtb/src/catalog/mod.rs:758-763) each list the root, status and directory pages of the commit log. A new kind of commit-log page added at one site leaves the others mapping or checking without it.

Fix direction: one `CommitLog::pages()` the three call.

Found in the quality review of `reviewed..main` at 7bf40103.
