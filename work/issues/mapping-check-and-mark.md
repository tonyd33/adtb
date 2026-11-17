---
created-at: 2026-11-17
status: open
kind: feature
priority: 130
summary: "the damage mapper judges a record or the bitmap outside the lock that publishes it, so a mark or a Database mark rests on a stale judgement"
tags:
  - engine
  - class-not-atomic
---

# Feature: Mapping check and mark

## Request

## Proposal

## Open questions

## Request

Mechanism for class-not-atomic group C, from the class count of 2026-11-17. crates/adtb/src/storage/damage_log.rs confirms a record current, then sets its marks, while another thread may free or restart it between; storage/quarantine.rs reads the bitmap only after the walk, so a page allocated during the walk reads as unreached and marks the database.

## Proposal

Set marks atomically with the record's epoch check, or defer a mapped page's record drop until its marks are set; read the bitmap before and after the walk. Enforcer: searching interleaving tests with reach floors, as damage_race.rs does.

Exit: mapped-record-superseded-before-mark and mapping-reads-bitmap-only-after-walk resolved; verify-changing-indexes-log-gap reproduced and placed.
