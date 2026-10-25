---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 501
summary: "no heap: no slotted pages, version header, TIDs, version chains or retag across heaps"
closed: 2026-10-25
depends-on:
  - page-buffer-contract
tags:
  - engine
---

# Feature: per-constructor slotted heaps and the version header

## Report

Implement `docs/design/mvcc.md` §1 to §3, §5, §6 and §12.1 to §12.5:
one slotted heap per constructor with heap ids, TIDs, the 24-byte
version header, the `next` TID and moved flag, version chains, retag
across heaps, the row body and large values. Settle `mvcc.md:29`, the
structure of a heap's page directory. Tested against a `BTreeMap`
model under simulation.

Prerequisite: `page-buffer-contract.md`. Visibility of a version is
`commit-log-snapshots.md`.

## 2026-10-25: resolved

Landed as `feat(heap): add slotted and fixed-slot heaps, version chains, retag and large values`. The directory is a tree of free space map pages with a fixed root (mvcc.md §1). Rows are `Vec<adtb::key::Value>` described by the heap's own `FieldType` (§12.2).
