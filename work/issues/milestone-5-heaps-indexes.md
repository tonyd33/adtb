---
created-at: 2026-10-25
status: open
kind: feature
priority: 500
summary: "milestone 5 (heaps and indexes) has not exited: no heaps, key encoding, B-link trees, vacuum or verify"
depends-on:
  - vm-root-race
  - latch-bound-overruns
  - blink-test-heights
  - blink-follow-ups
  - heap-concurrency-follow-ups
  - verify-depth
  - blink-page-deletion
  - heap-row-contract
  - heap-performance
  - blink-incomplete-split
  - blink-superlinear
  - blink-space-reuse
  - milestone-4-pager-log
  - slotted-heap
  - hash-anonymous-types
  - key-encoding
  - blink-tree
  - vacuum
  - verify-structural
tags:
  - milestone
---

# Feature: Milestone 5, heaps and indexes

## Report

Milestone 5 of `docs/plan.md`, on the critical path. Exit: model-based
tests against `BTreeMap` under simulation; `verify` clean after every
crash. Needs milestone 4 for the crash half of the exit; its tickets
start once `page-buffer-contract.md` is done.

Open on 2026-10-25: `slotted-heap.md`, `hash-anonymous-types.md`,
`key-encoding.md`, `blink-tree.md`, `vacuum.md`,
`verify-structural.md`.
