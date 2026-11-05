---
created-at: 2026-11-04
status: open
kind: bug
priority: 520
summary: "check's cargo calls run without --locked, so a stale Cargo.lock or apps/campaign/Cargo.lock is rewritten by a check"
---

# check rewrites a stale lock file

## Report

## Report

devenv.nix `scripts.check` runs `cargo clippy --manifest-path "$app"`, the root cargo builds and the suite without `--locked`. When a lock file is stale, a check rewrites it, a tracked file, and passes on the rewritten lock. Found in the review of devex speed, 2026-11-04, where check-reach had the same defect and was fixed.

## Proposal

Pass `--locked` to every cargo call in `check`, `suite` and the tools it runs, so a stale lock fails the check instead of being rewritten.
