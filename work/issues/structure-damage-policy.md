---
created-at: 2026-11-07
status: open
kind: feature
priority: 14
summary: "damage policy is split over four layers: two holders functions state opposite rules for the roots heap, 49 sites set or read marks, open bypasses the commit-ordered mark"
---

# Feature: One owner for damage mapping and marks across storage, write, catalog and api

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-exec.md (local). write/quarantine.rs:48 says roots heap damage marks nothing; catalog/mod.rs:850 says it marks the database. 3 open-time sites call `pager.quarantine` directly. The quarantine rewrite is applied by hand at 3 sites, and api/statements.rs:113-114 must read `ends_transaction` before it. The gated damage findings walk-damage-marks-database, unread-walk-marks-database, migrate-under-mark-silent and migrate-applies-under-mark are re-planned here.

Exit: one mapping function and one mark setter, the policy in storage.md §8 only, the rewrite applied in one place.
