---
created-at: 2026-10-25
status: open
kind: feature
priority: 599
summary: "duplicated helpers, hand-rolled byte reads and oversized functions found by the quality survey"
tags: [quality]
---

# Feature: quality refactor pass

## Report

From the 2026-10-25 survey. Duplicated: bound in plan/terms.rs and write/fresh.rs; repr and ctors in exec/types.rs and catalog/layout.rs; verify in heap/handle.rs and heap/large.rs; float in exec/prim.rs and exec/sink.rs; bytes in exec/prim.rs and prim/textual.rs; the commit sequence after Put::Done three times in heap/handle.rs. 28 hand-rolled little-endian reads in 13 files beside the readers in storage/header.rs and storage/record.rs. Split check/expr.rs build_ctor by phase, evolve/revert.rs inverse (615 lines), and check/prelude.rs prelude_call with a const-generic arity that removes its 31 unreachable!(). Unused pub fns: heap/handle.rs pointers_at, mvcc/txn.rs with_holds, visit.rs each_block_expr.

Also: deny `unwrap_used` once the byte reads go through the readers (60 sites in src; test helpers under `tests/` need `#[expect]` or a test-support conversion, 272 sites).
