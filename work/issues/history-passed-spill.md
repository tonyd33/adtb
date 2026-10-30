---
created-at: 2026-10-29
status: open
kind: feature
priority: 620
summary: Spill the deleted rows a read of the past has passed instead of refusing at the grant
---

# Feature: Spill the deleted rows a read of the past has passed

## Request

Since the exec-scan track (2026-10-29), a read of the past charges its set of passed deleted rows
(per inserter and deleter pair) to the call grant. Over many retained deleted rows with a small
grant it now answers `StatementLimit Memory` where it used to succeed. Spill the set to temporary
storage past the grant, as other operators spill, so the read succeeds.
## Proposal

## Open questions
