---
created-at: 2026-11-04
status: open
kind: feature
priority: 520
summary: "security.md 8.3 promises an audit ledger with digest and verify; the engine has none, so a dump carries audit rows only"
---

# Feature: Audit ledger is unbuilt

## Request

security.md 8.3 describes a ledger over `catalog::audit`, with `adtb ledger digest` and `adtb ledger verify`, and interop.md 6.1 dumps "`catalog::audit` with its ledger". The engine has no ledger, so a dump's `audit` entry carries rows only (dump S5.2, 2026-11-04). When it is built, make its digest independent of the file id, so a restore onto a new file id copies the ledger as it is and it still verifies.

## Proposal

## Open questions
