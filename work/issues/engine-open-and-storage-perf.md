---
created-at: 2026-10-26
status: resolved
closed: 2026-11-03
kind: feature
priority: 110
summary: "engine open re-parses and rebuilds every declaration, page checksums have no hardware CRC, reserve_page reads a page just for its LSN, and some sim seed loops skip profile::sweep"
tags:
  - engine
  - sim
  - test
---

# Feature: Engine open and storage perf follow-ups

## Report

Four perf gaps found while timing the suite:

- Engine open is still most of a differential case's time. `evolve`'s `Source::built` re-parses every declaration's text and runs `build_ahead` from `Cx::checks_where_written` per operator, quadratic in the number of operators. `Logged::names`, `Catalog::replay` and `fill` are next. Caching parsed declarations inside `Source` is the likely fix.
- CRC-32C is about 24% of the heap crash test, mostly whole-page verify on read. A hardware implementation needs `std::arch` in `adtb-sys`, since adtb forbids unsafe elsewhere, and a contract in its design file.
- `reserve_page` reads a free page from disk only to learn its previous LSN, with eviction, checksum and verify all on that path.
- The fixed seed loops in `adtb-sim` tests `faults.rs`, `ssi.rs` and `serial.rs` do not go through `profile::sweep`, so they do not scale with the profile. Low priority.

## 2026-10-30: measured

Caching parsed declarations targets 1.8% of suite cycles, so the fix is a memo on `Source::built`. CRC is 3.4% of the suite; 24% holds only for crash sweeps. `reserve_page` is 2.1%. The seed loop item is stale and dropped.

## 2026-10-30: hardware CRC

`adtb_sys::crc32c::update` uses the CPU instruction on x86-64 and aarch64, the table as fallback and under Miri. Same bytes on disk: 40800 differential cases against the old code, no difference. Cycles on two crash sweeps -18.5% and -12.1%. Not done: no test fails when `update` always takes the table; the aarch64 instruction was compiled but never executed; no file written by the old binary was opened.

## 2026-10-31: next step

Hardware CRC landed. What remains is the memo on `Source::built` and `reserve_page`'s read of a free page, each to be measured against a stop line first. The item on seed loops outside `profile::sweep` goes with test-audit-findings. No track is running.

## 2026-11-03: resolved

The memo on `Source::built` landed in `perf: memoise schema builds per source and scan visibility map leaves by set bit`. Ruling on `reserve_page`: dropped. Allocation logs a diff against the free page's current bytes (`Pager::allocating`), so the page is read for that diff whether or not `reserve_page` reads it first; the early latch only moves the read. Skipping the read needs a full-page image with an unknown previous LSN, which under docs/design/storage.md §8 clears damage and continues no chain, weakening the lost-write check `verify` runs, for 2.1% of suite cycles. Reopen if a profile shows allocation reads above 5% of a workload that matters.
