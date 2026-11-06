---
created-at: 2026-11-05
status: open
kind: bug
priority: 290
summary: "damage is made durable by whichever caller's error path carries it (failed open, verify's quarantine_findings, a query's quarantining), so each path that forgets to mark absorbs it"
tags:
  - class-damage-unreported
---

# Mark damage durably where it is detected

## Report

## 2026-11-05

The mechanism ticket of the class "Damage reported as nothing" (defect-classes.md), from the damage-reporting audit and the verify-fatal review. Today detection and durable marking are separate: recovery records damage in memory and only a failed open marks it (`mark_open_damage`); verify marks through `quarantine_findings`; a query marks through `quarantining` on the error path that carries it; the commit-log quarantine lives in `Transactions` memory. Each path that forgets is a new instance: recovery-damage-unmarked, verify-marks-whole-returns-fatal and its `verify r` hole, vacuum-erases-lost-write.

Rule to adopt through the design space: the component that detects damage (`Pager::damage`, the commit-log quarantine, recovery) makes it durable at detection, and findings and refusals are read from that one record; no caller marks. Decide with damage-readin-gate, which supplies detection on every read-in. Then delete the caller-side marking and re-check every ticket tagged class-damage-unreported against it.
