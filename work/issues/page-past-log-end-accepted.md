---
created-at: 2026-11-04
status: resolved
closed: 2026-11-04
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

## 2026-11-04: fixed

`fix: refuse a page whose LSN is past the log end on every read`. One predicate, `Damage::past_log_end`, refuses a page whose file copy is at or past `Pager::file_end(pid)`: the end of the log recovery kept, or the log's next LSN once this open has written the page out (`Meta.wrote`, a bitset set in `write_out`, never cleared). `resident`, backup's `sound`, replay and verify's judge all use it; a blank page (LSN 0) is exempt; a full-page image that follows in replay heals the page. storage.md §3 and §7 state both bounds. Class: damage accepted between two checks. Tests: a 12-entry sweep (query, each write form, vacuum, reindex, dump, verify, scrub, backup) over a planted page, the lost-tail test reading every page with and without appends past it, boundary probes at end - 1 and end, replay refusing and healing. The background worker is back on in `verify::a_page_past_the_log_end_is_an_unlogged_finding_of_its_relation`. Review found the next-LSN bound let lost-tail pages through once appends passed them; closed by the recovered bound. Full check 3396 passed.

Left: restore-launders-stale-page.
