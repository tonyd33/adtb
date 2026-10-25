---
title: CockroachDB encryption at rest
---
A store key wraps data keys; AES-CTR over every store file; data keys
rotate and old files are rewritten by compaction churn, not eagerly;
backups are not covered.
https://docs.cockroachlabs.com/docs/stable/security-reference/encryption
