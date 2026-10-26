---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 905
summary: "two contracts consumers rely on sit in non-binding notes: the heap row type and the isolation history format"
branch: adtb-verify
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
