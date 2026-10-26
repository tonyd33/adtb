---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 720
summary: no executor runs a physical plan over batches
closed: 2026-10-25
depends-on:
  - planner
  - batches
  - slotted-heap
  - blink-tree
  - key-encoding
tags:
  - engine
---

# Feature: bytecode pipeline executor and morsel scheduler

## Report

Implement `docs/design/execution.md` §7.1 and §7.5: the bytecode
pipeline interpreter over batches, push-based morsels, the scheduler
and memory accounting. Settle `execution.md:121`, how a plan names a
user-defined function's code, or refuse such plans with an error
naming them.

Prerequisites: `planner.md`, `batches.md`, `slotted-heap.md`,
`blink-tree.md`, `key-encoding.md`.

## 2026-10-25: resolved

Landed as `feat(exec): run physical plans as bytecode pipelines over morsels at a snapshot`. 47 running-schema queries under 11 rewrite variants match the reference over real heaps and indexes. Follow-ups: `exec-primitives.md`, `exec-follow-ups.md`, `spec-memory-limit.md`.
