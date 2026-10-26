---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 905
summary: "two contracts consumers rely on sit in non-binding notes: the heap row type and the isolation history format"
closed: 2026-10-25
tags:
  - design
---

# Feature: Move contracts out of the notes

## Report

`docs/design/README.md` says "Notes for the implementation" sections
are not binding. Two of them now hold contracts another subsystem
relies on:
- `runtime.md` §6.4, the history format between the test harness and
  `adtb-isolation`;
- `mvcc.md` §12.2, the heap's row type (also `heap-row-contract.md`).

Move each contract to a binding section of its file, and keep the
implementation detail in the notes.

## 2026-10-25: resolved

Landed as `feat(verify): check every page, heap, tree and index entry and free leaked pages`. The verify contract is `docs/design/verify.md`; the row type is `mvcc.md` §1.1 and the history format `runtime.md` §6.
