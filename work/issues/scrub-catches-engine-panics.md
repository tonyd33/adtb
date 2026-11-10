---
created-at: 2026-11-10
status: open
kind: bug
priority: 400
summary: "scrub catches engine panics, turns them into finding text and keeps serving, against the fatal-split ruling"
tags:
  - class-failure-flattened
---

# Scrub catches engine panics

## Report

## 2026-11-10

api/maintenance.rs:430 wraps scrub's page reads in a panic catch and records the panic's text as a finding, then keeps the engine serving. The fatal-split ruling ends the process on an engine invariant. structure-transactions (ruling Q2) keeps this catch until typed shape faults cover every page kind scrub reads, with every resource inside the catch a guard after that program's step 3; reindex's catch goes in that program.

Direction: list what scrub reads that can panic on a malformed page today; give each a typed `Damage::Shape` refusal; then remove the catch so a panic ends the process.

Defect class: Damage or failure gives a panic, hang or flattened error.
