---
created-at: 2026-11-18
status: open
kind: feature
priority: 270
tags: [test]
summary: fault sweeps assert the success value but not the exact error; storage refusals carry their cause only as text
---

# Feature: Fault sweep causes

## Request

From the refusal-sites review, finding 3. Six fault sweeps (backup_tests.rs 2248, 2420, 2535, 2701, 2780; dump_tests.rs 1903) assert the success value exactly but not the error, because `FatalReason::Storage { detail }` names whichever operation failed as text. Proposal: `each_failure` passes the injected IoError to the check, and the storage refusal carries that cause as a typed field, so a sweep asserts `Fatal Storage` with that exact cause.
