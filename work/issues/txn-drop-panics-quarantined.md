---
created-at: 2026-10-30
status: open
kind: bug
priority: 428
summary: "dropping an open transaction on a database with a damaged commit log panics: the assert in api/txn.rs accepts only Fatal Storage, the abort returns Fatal Quarantined"
---

# Txn drop panics quarantined

## Report
