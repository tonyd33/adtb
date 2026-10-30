---
created-at: 2026-10-30
status: in-progress
branch: open-damage
kind: bug
priority: 428
summary: "dropping an open transaction on a database with a damaged commit log panics: the assert in api/txn.rs accepts only Fatal Storage, the abort returns Fatal Quarantined"
---

# Txn drop panics quarantined

## Report

Reproduced in review of quarantine-gate: `api/txn.rs` near line 282. Acceptance: the drop neither panics nor loses the error, by a test on a damaged commit log page.
