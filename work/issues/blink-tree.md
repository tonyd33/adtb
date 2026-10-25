---
created-at: 2026-10-25
status: open
kind: feature
priority: 503
summary: "the B-link tree is single-threaded: no optimistic lock coupling over the latches"
depends-on:
  - vmcache
  - page-buffer-contract
  - key-encoding
tags:
  - engine
---

# Feature: B-link trees with optimistic lock coupling

## Report

Implement `docs/design/indexes.md` §1, §6 and §8.1: B-link trees with
optimistic lock coupling over the buffer pool, constructor-set indexes,
indexes on value-level sum fields, hash indexes. Loom tests for the
latch protocol; model-based tests against `BTreeMap` under simulation.
Settle `indexes.md:187` (who chooses a hash index); `indexes.md:269`
stays deferred.

Prerequisites: `page-buffer-contract.md`, `key-encoding.md`.

## 2026-10-25: single-threaded tree landed

`feat(btree): add B-link tree indexes, order keys and anonymous key
types` adds `adtb::btree`: leaf and inner pages, splits with high keys
and right links, a root that never moves, range and prefix cursors,
constructor-set indexes, value-level sum field indexes, and `verify`,
over the single-threaded pager. What remains is optimistic lock
coupling over the latches of `docs/design/storage.md` §2.4, once
`vmcache.md` lands, with its tests.
