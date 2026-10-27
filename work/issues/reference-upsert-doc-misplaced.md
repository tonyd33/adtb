---
created-at: 2026-10-26
status: open
kind: bug
priority: 774
summary: "file_records was inserted under upsert's doc comment, so it carries two docs and upsert none"
tags:
  - quality
  - reference
---

# file_records carries upsert's doc comment, and upsert has none

## Report

`crates/adtb-reference/src/stmt.rs:509-555`: the new `file_records` was inserted between `upsert`'s doc comment and its body, so `file_records` opens with "Evaluates the upsert `t` over its proposed `rows` ... Returns its binding." before its own doc, and `fn upsert` (stmt.rs:555) has none. The doc of `records` (stmt.rs:439) still says "Over a source relation" and does not mention the file scan branch.

Fix: move the upsert lines back above `fn upsert`, and extend the `records` doc to cover file scans.
