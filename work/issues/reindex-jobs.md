---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 411
summary: "reindex does not run; no sorted-run build, and a quarantined index has no way back"
closed: 2026-10-30
---

# Feature: Reindex jobs

## Request

Build slice S12 of the milestone 9 audit: a sorted-run build in `btree/`, offline and online rebuild jobs with `sys::jobs` rows, and `reindex r index (f)` with no `in S` (ruling ops/reindex/clause/in-disambiguates). `reindex r` includes the key index and uniques. Un-ignore the two reindex tests of `crates/adtb-sim/tests/commands.rs`. Acceptance: a quarantined index is usable again and rebuilds survive writers and a crash at each stage. quarantine-release lands in the same track.

## Proposal

## Open questions

## 2026-10-30

Landed on main. Check passed, 3216 tests. Follow-ups: reindex-job-holds-engine, reindex-build-memory, reindex-catch-up-cost, reindex-bulk-no-stop.

## 2026-10-30: resolved
