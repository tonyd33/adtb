---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 503
summary: "no index: no B-link tree, latch protocol or constructor-set index over it"
branch: adtb-blink
depends-on:
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
