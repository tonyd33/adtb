---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 880
summary: the spec does not say how a call whose owned request is larger than the whole admission queue bound is refused; the engine answers StatementLimit Memory with the bound
closed: 2026-10-29
---

# Bug: the spec leaves a call larger than the admission bound unstated

depends on: none
needed by: none

## Report

From the result/task plan (server-owned-call-tasks, ruling Q7). catalog.md section 8.1 and wire.md section 8 answer a full queue with `Retry { reason: Overloaded }`, which a client retries. A call whose owned arguments are larger than the whole queue byte bound can never be admitted. The engine answers `Rejected { reason: StatementLimit { limit: Memory { bytes } } }`, `bytes` the admission byte bound. Take it through spec-revision: state it in catalog.md 8.1 and wire.md 8, or choose another non-retryable reason.

## 2026-10-29: closed

Stated by spec revision 30, landed at a478f4ba. Review: work/briefs/review-spec-rev30.md, four rounds. Branch check 2828 passed, 7 skipped, campaign 47. Code gaps: grant-spec-rev30-code.

## 2026-10-29: resolved
