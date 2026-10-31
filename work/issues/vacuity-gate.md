---
created-at: 2026-10-31
status: open
kind: bug
priority: 403
summary: a test that cannot fail lands unless a reviewer happens to notice
---

# Vacuity gate

## Report

About 20 tickets record tests asserting less than they claim. Hand mutants are listed in receipts, and `cargo-mutants` runs only at a milestone exit.

Add `cargo-mutants` to the dev shell and run it on the diff in the branch check, with a survivor needing a written reason. Measure its time first against a stop line in the brief. Add one helper asserting an exact refusal, and refuse `is_err` and substring refusal assertions in new tests.

Replaces the assertion gaps of bundles-test-assertions, command-test-assertions, embedding-test-assertions, file-scan-test-assertions, stale-value-declared-untested, uring-fallback-test-tautological going forward. Fixing those tests remains their own work.
