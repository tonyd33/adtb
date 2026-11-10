---
created-at: 2026-11-09
status: open
kind: feature
priority: 380
summary: "the damage search injects one fault per case, so holes that need two faults (a lost write and a log gap, an unreadable list page and a crash mid-fold) are found only by review; add a tier of fault pairs judged against 'no damage record and no durable mark is ever lost'"
tags:
  - testing
---

# Feature: fault pairs in the damage search

## Request

structure-damage-policy steps 4b and 5 each removed a mechanism that also covered an unstated case, and review found the holes by reasoning through sequences of two faults: records the list could not fold recycled with their log, a replayed free lost with the list page unreadable at open, a fold's bound cleared before its change was logged, a marks page loaded unverified written back over a lost mark. Each fix is pinned by one test for its scenario. The damage search (crates/adtb-sim/tests/storage/damage_search) injects one fault per case and cannot reach any of them.

## Proposal

A tier of the damage search that injects pairs: a lost or recycled write of one page with a log gap, an unreadable damage list or marks page, or a crash at each point of a checkpoint's fold. Judge two properties over every case, across closes, crashes and reopens: every damage record logged reaches the damage list or a durable mark, and every durable mark survives until released. The tier runs in a deep profile with a small default count. Schedule before structure-damage-policy closes, after its step 11.
