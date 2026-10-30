---
created-at: 2026-10-30
status: in-progress
branch: quarantine-gate
kind: bug
priority: 437
summary: "removing the vacuum hold in check_indexes passes every test, including a 25 s real-thread run; show the hold is needed or remove it"
---

# Verify vacuum hold untested

## Report

`check_indexes` in `write/verify.rs` holds off vacuum during its run. Removing the hold passes every test. Acceptance: a test that fails with the hold removed, or the hold is gone with the reason it is unneeded in the report.
