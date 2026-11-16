---
created-at: 2026-11-16
status: open
kind: bug
priority: 110
summary: a key naming a set the same migration adds later panics in debug and is stored with an unqualified name in release
tags:
  - lang
  - class-identity-by-spelling
---

# Late set name unresolved in canon

## Report

Found by the review of committed-schema-type part 2b-m-2 (2026-11-16), reproduced on its tree and not touched by it. A valid first migration in the shape evolution.md §3.1 allows:

```
migrate
  module m
  add module m
  add type W { id: Key ws, p: Option (Key ws.Settled) } = A | B
  add relation ws : W key id generate sequence
  add set Settled of ws = B
run
  insert ws.B { p: None }
```

panics in a debug build: "every type, constructor and named set a term names resolves, and `Settled` at 65..72 of "type W {...}" does not" (crates/adtb-lang/src/canon.rs:202). A `define fn` naming `Key ws.Settled` before the set panics the same way. In a release build `spelled` (canon.rs:226-230) falls back to the written text, so the stored canonical text holds `Settled` where it should hold `m::Settled` and no catalog id is recorded for the name; consumers are catalog/rows.rs:179 (index element texts) and evolve/resolved.rs:564 (function code). A wrong stored identity, not a refusal.

Fix direction: `Names` records a late-selected name's module and id once the set resolves at the migration's end. Ruled into committed-schema-type part 2b-ii (names per declaration). A test per declaration kind naming a set added later, asserting the stored canonical text whole.
