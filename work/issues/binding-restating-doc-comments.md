---
created-at: 2026-10-31
status: open
kind: bug
priority: 899
summary: build_stop and RoleBinding::binding carry doc comments that restate their names
tags:
  - quality
  - engine
---

# Binding restating doc comments

## Report

crates/adtb/src/write/reindex.rs:521 `/// Returns whether a build should stop.` on `build_stop`, and crates/adtb/src/api/server/binding.rs:307 `/// Returns what the bind observed.` on `binding`, restate the name. Remove both.
