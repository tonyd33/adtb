---
created-at: 2026-10-27
status: open
kind: feature
priority: 872
summary: "the checker types an at body at the current schema, takes no pins on an at block, and nothing raises WrongVersion"
tags:
  - lang
---

# Feature: check at blocks at their version

## Report

`derived.md` section 6.3: an `at` block's body is typed at the schema of the position it names, `pins` fix that version, and a read whose relevant types differ from the pinned version's is refused with `WrongVersion`. The checker types every `at` body at the current schema and accepts no `pins`.
