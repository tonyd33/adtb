---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 880
summary: the spec does not say how a call whose owned request is larger than the whole admission queue bound is refused; the engine answers StatementLimit Memory with the bound
branch: spec-rev30
---

# Bug: the spec leaves a call larger than the admission bound unstated

depends on: none
needed by: none

## Report

From the result/task plan (server-owned-call-tasks, ruling Q7). catalog.md section 8.1 and wire.md section 8 answer a full queue with `Retry { reason: Overloaded }`, which a client retries. A call whose owned arguments are larger than the whole queue byte bound can never be admitted. The engine answers `Rejected { reason: StatementLimit { limit: Memory { bytes } } }`, `bytes` the admission byte bound. Take it through spec-revision: state it in catalog.md 8.1 and wire.md 8, or choose another non-retryable reason.
