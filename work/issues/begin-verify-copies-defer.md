---
created-at: 2026-11-11
status: open
kind: bug
priority: 500
summary: "begin_verify copies the deferral loop of begin_read and defer, and the copies differ on an empty pending-writer list: one yields, the other indexes writers[0]"
tags:
  - quality
  - engine
  - class-copied-rules
---

# begin_verify copies the deferral loop and the copies differ

## Report

`begin_verify` (crates/adtb/src/write/verify.rs:171-193) repeats the loop of `begin_read` and `defer` (crates/adtb/src/write/mod.rs:833-870): decide safety, abort and begin again when unsafe, wait on a pending writer. On an empty pending-writer list `begin_verify` yields and loops, while `defer` takes `writers[0]` and would panic. Nothing states whether the serializer can return `Pending` with no writers.

Direction: one loop, `begin_verify` built on `begin_read(true)` after its `outside_writer` check; state the empty case once (a type that cannot be empty, or one handling).

Defect class: Wildcards and copied rules.
