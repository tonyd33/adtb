---
created-at: 2026-10-25
status: open
kind: feature
priority: 720
summary: no executor runs a physical plan over batches
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
