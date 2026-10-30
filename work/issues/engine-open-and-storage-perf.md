---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 110
summary: "engine open re-parses and rebuilds every declaration, page checksums have no hardware CRC, reserve_page reads a page just for its LSN, and some sim seed loops skip profile::sweep"
branch: schema-memo
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
