---
created-at: 2026-11-09
status: open
kind: bug
priority: 340
summary: "an open with the read-in gate off reads pages with no expected-LSN check and can still write them back under a fresh LSN, so the next armed open accepts a stale copy as current"
tags:
  - class-damage-unreported
---

# Bug: a page read unchecked can be written back

## Observed

When the log cannot arm the read-in gate (a gap, an undecodable batch, a `load_log` failure), the open holds the database and reads pages with no expected-LSN check (storage.md §10.3). Any write of such a page in that open stamps the stale copy with a fresh LSN, and the next open with the gate armed accepts it. structure-damage-policy step 5 closed this for the marks page (`Marks::writable_page`) and the damage list (no fold while unverified). Other writes remain, for example the free-space bitmap written by a swap in a gate-off open (recycle_tests `a_swap_releasing_a_mark_on_an_unchecked_marks_page_keeps_the_mark`).

## Expected

One rule at the pager: a page read while the gate is off is never written in that open, or is written only after it is checked against the retained log. Every write path goes through it.

## Reproduction

Found by the delta review of structure-damage-policy step 5. The damage-search-fault-pairs tier (a lost write with a log gap) should reach it.
