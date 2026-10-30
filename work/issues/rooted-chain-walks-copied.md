---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 720
summary: "rooted.rs walks a heap-only chain twice and repeats scan_pages, without the HEAP_ONLY test prune applies to a link"
closed: 2026-10-30
tags:
  - quality
  - engine
---

# rooted.rs walks a heap-only chain twice and repeats scan_pages, without the test prune applies to a link

## Report

`crates/adtb/src/heap/rooted.rs:29` (`scan_rooted`) and `:92` (`chain_rows`) each follow `next` through `HOT_UPDATED` slots, and `crates/adtb/src/heap/prune.rs:169` `links` and `:192` `chain` do the same over parsed slots. The second loop of `scan_rooted` (`rooted.rs:49`) repeats the body of `scan_pages` at `crates/adtb/src/heap/handle.rs:438`.

The copies disagree on what a link is: `prune::links` requires the successor to be `HEAP_ONLY`, and the two walks in `rooted.rs` do not. A sound page holds them equal, as `handle.rs:1343` checks. A damaged page is the input of verify, the one caller of `rooted.rs`, and by reading there a stray `next` adds an unrelated row to `chain_rows`, so an orphan entry passes, or gives a row the wrong root, so `check_rows` reports a false `Missing`.

Fix direction: one chain walk on a latched page, used by all three, with the `HEAP_ONLY` test. Have `scan_rooted` compute roots, then share the version loop with `scan_pages`. Test a `next` naming a slot that is not heap-only.

## 2026-10-30

Fixed in the quality-fixes32 commit; check passed with 3023 tests.

## 2026-10-30: resolved
