---
created-at: 2026-10-25
status: open
kind: feature
priority: 601
summary: "no transaction ids, commit log, snapshots, visibility rule or watermark"
depends-on:
  - page-buffer-contract
tags:
  - engine
---

# Feature: commit log, snapshots and visibility

## Report

Implement `docs/design/mvcc.md` §4, the visibility half of §5, and §9:
64-bit xids, the commit log, snapshots, the visibility of a version
header to a snapshot, and the watermark that bounds vacuum.

Prerequisite: `page-buffer-contract.md`, against which the commit log
is paged.

## 2026-10-25: from the pager

`adtb::storage::Recovered::committed` reads commit records from the
log. Once the commit log exists, recovery should report commits from it
and drop that field.
