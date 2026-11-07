---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 430
summary: "a Damaged record logged after its page's Freed outlives the reallocation, so the new owner's healthy page is reported and an API verify marks it"
closed: 2026-11-07
tags:
  - class-damage-unreported
---

# A damage record logged after its page's free outlives the reallocation

## Report

## 2026-11-06

From the damage-log delta review (work/briefs/review-damage-log-r1-delta-report.md, D2), reproduced with a sequential stand-in for the race. A detection of page P concurrent with the batch freeing P can log `Damaged { P }` after `Freed { P }`; the entry then survives P's reallocation, and the new owner's healthy page is reported by verify and marked by an API verify. Needs the page's evidence absent from the file. Direction: order the free and the record under one lock, or drop a record whose page is free at fold. Defect class: Damage reported as nothing (here, reported on the wrong page).

## 2026-11-07: fixed

Fixed by `feat: start a page's log chain at allocation and log every free` (page-incarnation-identity, M1). Every free logs `Freed`; a Damaged detection between a `Freed` and the next chain start is held in memory, mapped in its open and never logged, and the chain start drops every entry of the page. Test: the incarnation search's damage-after-free and recorded-after-recycle cases (storage incarnation.rs).

## 2026-11-07: resolved
