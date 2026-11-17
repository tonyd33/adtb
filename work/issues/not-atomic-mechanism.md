---
created-at: 2026-11-16
status: resolved
closed: 2026-11-17
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

## Resolution 2026-11-17

Counted by a read-only track. Replaced in the class table by commit-point (group A: durable commit then a fallible step), retained-guard (group B: side update before a fallible main update) and mapping-check-and-mark (group C: judged outside the lock that publishes). Not this class or stale: concurrent-rebuilds-share-marker is unreachable on main (reindex serializes through `holding_off_vacuum`; latent, a site fix makes `begin_reindexing` take `&VacuumHold`); call-results-failure-ends-transaction is spec conformance, a site fix; verify-changing-indexes-log-gap is unreproduced, likely a read without a stable log bound.
