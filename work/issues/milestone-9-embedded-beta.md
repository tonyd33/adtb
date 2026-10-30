---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 900
summary: "milestone 9 (operations, the embedded beta) has not exited: statistics and plan pinning, explain, config and metrics, dump, restore and backup, and the published benchmarks are open"
depends-on:
  - milestone-8-embedded-alpha
  - statistics-and-plan-pinning
  - dump-restore-backup
  - ops-config-resource-metrics
  - fs-fault-real-backends
  - comparative-benchmarks
  - command-follow-ups
  - verify-online
  - quarantine-wired
  - scrub-worker
  - background-vacuum
  - compaction
  - reindex-jobs
  - activity-and-metrics
  - real-backend-faults-engine
  - format-settings
tags:
  - milestone
---

# Feature: Milestone 9, operations, embedded beta

## Request

Milestone 9 of docs/plan.md. Exit: the suite and every test method pass; benchmarks are published with the release. Tony, 2026-10-29: it may run out of order, but it must not be skipped. It had no ticket until now. vacuum and verify-depth are resolved. Before dispatching a slice, audit each docs/plan.md bullet against the code and the manual's list of what the engine still refuses.
## Proposal

## Open questions

## 2026-10-29: audit

work/briefs/review-m9-audit.md: 6 of 53 requirements done, 14 partial, 33 absent. Order: verify-online, quarantine-wired, scrub-worker, dump-restore-backup (S4 to S6), then settings and the rest. Rulings adopted so far: spec/verify-structure/structure, spec/verify-scope/per-scope, spec/tool-name/adtb; OpenTelemetry and traces move to milestone 10. The other rulings in the report are made when their slice is dispatched.
