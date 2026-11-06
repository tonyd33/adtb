---
created-at: 2026-11-06
status: open
kind: bug
priority: 430
summary: "a Damaged record logged after its page's Freed outlives the reallocation, so the new owner's healthy page is reported and an API verify marks it"
tags:
  - class-damage-unreported
---

# A damage record logged after its page's free outlives the reallocation

## Report

## 2026-11-06

From the damage-log delta review (work/briefs/review-damage-log-r1-delta-report.md, D2), reproduced with a sequential stand-in for the race. A detection of page P concurrent with the batch freeing P can log `Damaged { P }` after `Freed { P }`; the entry then survives P's reallocation, and the new owner's healthy page is reported by verify and marked by an API verify. Needs the page's evidence absent from the file. Direction: order the free and the record under one lock, or drop a record whose page is free at fold. Defect class: Damage reported as nothing (here, reported on the wrong page).
