---
created-at: 2026-11-16
status: open
kind: feature
priority: 119
summary: class-not-atomic has open instances and no live mechanism ticket since fault-matrix resolved
tags:
  - engine
  - class-not-atomic
---

# Feature: Not atomic mechanism

## Request

`class-not-atomic` lists fault-matrix as its mechanism ticket, and fault-matrix is resolved, so the class is ungated with open instances: call-results-failure-ends-transaction, mapping-reads-bitmap-only-after-walk, heap-write-settle-skips-release, large-retain-before-map-refusal, migrate-applies-under-mark, mapped-record-superseded-before-mark, verify-changing-indexes-log-gap. Found by the cause review of 2026-11-16.

## Proposal

Group the open instances by the step that publishes state before the work it depends on completes, and name one mechanism per group (build aside and publish last; memory updated under the latch; a fault sweep that reaches each publish point). The track that takes this counts first and proposes the split; it replaces this ticket in the class table with the mechanism tickets it files.

## Open questions
