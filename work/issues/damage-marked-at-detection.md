---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 290
summary: "damage is made durable by whichever caller's error path carries it (failed open, verify's quarantine_findings, a query's quarantining), so each path that forgets to mark absorbs it"
closed: 2026-11-06
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

## 2026-11-06 ruled: two changes

Change 1 makes each detection a durable page-level record; change 2 derives object marks from the records and deletes caller-side marking. Rulings for change 1: `Record::Damaged { pid, damage }` is not a page record and never raises an expected LSN; a record is durable at the next force, sound per site only while its evidence outlives it; a checkpoint folds records into a damage list page named in the file header, a later full-page image dropping its entry, damage to the list page holding Database for the open. Recovery's page damage goes in its closing checkpoint batch after `clear_tail`, re-detected after a crash before it. Recovery's past-end log findings are written to the file header and forced before `clear_tail`, then folded; they derive no mark and verify reports them. A settle records `Damage::Settled`, which derives no mark, so a settled commit-log page does not quarantine the file. The lock-order trace of every `damage` site is a receipt table the one fresh review checks.

## 2026-11-06: change 1 landed

`feat: record every detected page damage durably`. `Pager::damage` appends `Record::Damaged` under the log lock and forces it before returning, except during recovery before `start_recording`; `Unreadable` is never recorded. A checkpoint folds changed records into a damage list page named in the file header (24-byte entries; past capacity the whole file is recorded and verify reports `Unlisted`). A free drops an entry of any kind through `Record::Freed`; a full-page image after the record drops a non-Settled entry. Recovery writes stale log blocks past the end to the header and forces them before `clear_tail` (62 entries, an overflow bit past them). A settle records `Damage::Settled`, reported as `Found::Settled`, which never marks. On a file marked whole the list page may change: a checkpoint folds what the log holds, and later detections stay in memory. Callers still mark as before; change 2 derives the marks.

Review r1: blockers B1 (a checksum record lost after a crash when the page was written since the checkpoint), B2 (stale log findings past 62 dropped), B3 (a settled status page marking the database through verify), B4 (the site sweep claiming every site), and a record outliving a free; delta review D1 (a record forced before a whole-file mark lost at the next checkpoint). Each fixed with a test shown failing without its fix. 15 of 23 production `damage` sites are reached by a named test; the receipt lists the 8 others with reasons. Expectations changed by the contract: open_damage allows the list page to change on a whole file and keeps a Checksum row planted before the mark; catalog's migration test keeps a hand-repaired page's finding; 26 script instants moved with the list page's I/O. Filed: damaged-record-after-free, mark-change-crash-window. `check --changed` on 5fabbe16: suite 3468 passed, campaign 48 passed, house-check listing one ordering-invariant comment, kept.

## 2026-11-06: change 2 landed

`fix: derive quarantine marks from the damage records`. One mapper, `Pager::map_damage`, maps the records not yet mapped through `holders` and sets the marks; it runs at every open (success or failure), at the end of an API call that met damage (`quarantining`, `quarantine_findings`), and in a commit under `ser` before `admit_commit` (`map_committing`, mapping only the records unmapped when the hold began, and a page met by its own walk only when this thread's read detected it). Settled, log findings, `Unlisted` and `Unreadable` map to no mark; a list overflow, a damaged list page and a blank bitmap page of a started group map to Database. The caller-side page marking is gone; the failure a call returns names the quarantine from the marks. Index findings (Order, Orphan, Missing, OrphanEntry) still mark their index at the detecting call. Ruling (a): verify does not refuse a page recorded Settled at its settled LSNs and reports it once; a stale finding at other LSNs maps to Database. Review and delta review: a commit marking the whole database for another thread's concurrent damage (fixed: `Records::detected_by`), and four smaller findings, fixed. Full-suite tests changed to the contract: a lost bitmap group now marks {Database, users heap}, as storage.md's failure table states; a history page damaged during a reindex is marked at the detecting call's end, not at detection, and the reindex's full-page image drops the record and releases it. `check --changed` 3477 of 3478 passed, the one timeout btree-crash-test-slow; full `check` on main after landing.

Closes recovery-damage-unmarked and mark-change-crash-window (page damage). Still open from the class: damaged-record-after-free, verify-free-page-drops-chain, catalog-settled-inprogress, backup-list-misses-late-stale, incomplete-mapping-never-retried; damage held in memory only (a whole-file mark, a sound resident page) is not re-derived at open.

## 2026-11-06: resolved
