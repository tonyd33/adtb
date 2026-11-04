---
created-at: 2026-11-04
status: open
kind: bug
priority: 330
summary: A page whose LSN is past the log end is read and rewritten by any writer; Pager::resident checks only the older direction
---

# Pager accepts a page past the log end

## Report

`Pager::resident` (crates/adtb/src/storage/pager.rs) refuses a page as `Damage::Stale` only when its LSN is before the one expected. A page whose LSN is past the log's end, a lost log record by `Damage::Stale`'s own doc, is accepted: any writer (an `update`, `vacuum r`, the background vacuum) rewrites it, chaining its new record from the bogus LSN. `verify` later still reports `StalePage` and `Unlogged` from the log chain, but queries read the page as sound in the meantime. With the background vacuum, every reopen rewrites such a page with no command. `verify::a_page_past_the_log_end_is_an_unlogged_finding_of_its_relation` runs its reopen with the worker off until this is fixed.

Found in background vacuum V3, delta round 2, 2026-11-04 (seed 45, page 556 at LSN 2^40).

## Proposal

Refuse in `Pager::resident` a page whose LSN is at or past the log's next LSN, as `Damage::Stale`, so the page is quarantined on every read and write path at once. This changes what queries and `verify` report for such a page; check verify.md and the quarantine sections of catalog.md, then turn the worker back on in that test.
