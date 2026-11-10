---
created-at: 2026-11-10
status: open
kind: bug
priority: 360
summary: "verify_of_indexes_that_are_changing_finds_and_marks_nothing failed once with a Structure finding of a log gap"
tags:
  - class-not-atomic
---

# verify over changing indexes reported a log gap once

## Report

## 2026-11-10

`export quarantine::verify_of_indexes_that_are_changing_finds_and_marks_nothing` failed once in a full suite run during structure-damage-policy step 10 with `Structure "log at 7039488: Gap"`. It then passed 5 of 5 alone and in two full suites. The step's change does not touch verify's log read.

A `Gap` finding on a sound file is a false report of damage. A likely cause is verify reading the retained log while a concurrent commit or checkpoint extends or recycles it, with no snapshot of the log's bounds.

Direction: find what verify reads of the log while writers run and whether its bounds are taken once; reproduce under load (many iterations under `--profile deep` or a stress loop) before fixing.

Defect class: Multi-step change not atomic under failure, crash or race (assumed until the cause is found).
