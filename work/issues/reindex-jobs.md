---
created-at: 2026-10-29
status: open
kind: feature
priority: 411
summary: "reindex does not run; no sorted-run build, and a quarantined index has no way back"
---

# Feature: Reindex jobs

## Request

Build slice S12 of the milestone 9 audit: a sorted-run build in `btree/`, offline and online rebuild jobs with `sys::jobs` rows, and `reindex r index (f)` with no `in S` (ruling ops/reindex/clause/in-disambiguates). `reindex r` includes the key index and uniques. Un-ignore the two reindex tests of `crates/adtb-sim/tests/commands.rs`. Acceptance: a quarantined index is usable again and rebuilds survive writers and a crash at each stage. quarantine-release lands in the same track.

## Proposal

## Open questions
