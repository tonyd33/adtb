---
created-at: 2026-11-04
status: open
kind: bug
priority: 680
summary: "Dropping a role that does not exist answers Denied: the authenticated role no longer exists"
---

# Dropping an unknown role names the session

## Report

`drop role nobody`, for a role that does not exist, answers `Denied { detail: "the authenticated role no longer exists" }`. The lookup in crates/adtb/src/api/server/tables.rs serves both a session's own role and a named role, so a missing named role gets the session's wording and kind. The grantee lookup beside it answers `Invalid` naming the role. scripts/probes/txn-values.differs pins the current value.

## Proposal

Answer a missing named role as the grantee lookup does, naming it, and check the refusal kind against security.md.
