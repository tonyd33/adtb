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

## 2026-11-05 ruled

Adopted storage/damage-mark/damage-log (with derived-order/map-under-ser, record-full/overflow-database) and storage/readin-gate/open-map plus recycle (open-scan-cost/bound-by-recycle, backup-retention/live). Research: work/research/notes/damage-mechanism.md. Order: measure read_log per MiB first; if material, recycle lands with or before open-map; then damage-log. Briefs must carry: the map built by the coverage-checked scan, failing closed as LogUnproven; Unreadable never recorded; records kept in memory on a file marked whole; recovery records durable before clear_tail, with a crash search over that window; the map updated on every append (write-path cost named); every damage call site traced with its lock order and a scoped review; the holders walk under ser bounded. Recycle limits: never at or past redo, never a segment a running backup copies, never an unarchived one when an archive is set; the settle residual grows.

## 2026-11-05 settle is a detection

Under the revised recycle ruling, settling a status from a retained commit record means the status page lost a write: settle must record it as damage once the damage log lands, or recycling erases the only evidence verify had.
