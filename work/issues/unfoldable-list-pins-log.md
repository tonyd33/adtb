---
created-at: 2026-11-09
status: open
kind: bug
priority: 220
summary: "while the damage list page cannot be latched in session, or the open cannot read it, every checkpoint's redo LSN stays at the first record no fold has taken, so the kept log and what the next open replays grow without bound"
tags:
  - class-unbounded-work
---

# Bug: an unreadable damage list page pins the log

## Observed

While the damage list page cannot be latched in session, or the open cannot read it, every checkpoint's redo LSN stays at the first record no fold has taken (`Evidence::redo_bound`, crates/adtb/src/storage/evidence.rs). The log the session keeps, and what the next open replays, grow without bound until an open reads the list page and folds. Memory holds one record per page.

## Expected

A bound on the log kept for unfolded records: the database refused once the kept log passes a stated size, or the records written somewhere other than the list page, so a lasting list page failure costs a bounded log.

## Reproduction

`a_damage_record_the_list_cannot_fold_keeps_its_segment_and_reaches_every_later_open` in storage in_session_tests shows the pin. Found in structure-damage-policy step 4b, which stopped storage from holding the database at detection in session.
