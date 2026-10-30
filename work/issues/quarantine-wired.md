---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 410
summary: "nothing raises Quarantined: a bad page answers Fatal Storage, no mark is set outside tests, other reads of its constructor still run"
closed: 2026-10-30
---

# Feature: Quarantine wired

## Request

Slice of milestone 9; scope, tests and acceptance in work/briefs/review-m9-audit.md, S2. Depends on verify-online.

## Proposal

## Open questions

## 2026-10-30

Landed after three review rounds; check passed. A damaged page or a verify finding marks its holder durably, a call reading it fails `Quarantined`, a marked file opens for `verify`. A latch-free walk that only saw a shape it could not follow never marks. Still `OpenError::Corrupt`: catalog root, roots heap, commit log root and directory pages, and catalog relation pages the open reads. Follow-ups: quarantine-release, verify-damaged-catalog, ctor-stats-rows, quarantine-large-values, quarantine-commit-gate, quarantine-explain-index, quarantine-index-scan-cost, quarantine-history-index-read, status-damage-open-migration, verify-vacuum-hold-untested.

## 2026-10-30: resolved
