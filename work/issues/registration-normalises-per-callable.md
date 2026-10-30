---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 836
summary: the registration check and the migration recheck normalise the whole schema program once per callable of every bundle
closed: 2026-10-29
tags:
  - quality
  - engine
  - lang
---

# Registration normalises the program once per callable

## Report

`crates/adtb/src/api/registered.rs:314` `requirements` calls `permission::normal` (`crates/adtb-lang/src/permission/mod.rs:88`) for each query and transaction of a bundle. Each call runs `norm::program` over the whole schema program again. `lacking_of` (`registered.rs:294`) runs it at every role registration, at every open for each live nonowner bundle (`definers`), and at every migration recheck for each live nonowner bundle (`recheck`). The cost is callables times the size of the program, on the path that holds `bundle_writer` and on every open. The permission-provider follow-ups fixed this same per-statement repeat for ad hoc text.

Fix: take a `&NormalProgram` in `permission::normal`, normalise once per schema and program in `lacking_of` (once per `Over` and per translation target), and pass it to every callable.

## 2026-10-29: closed

Fixed by track quality-fixes30, landed at a3e00f73. Receipt: work/briefs/receipt-quality-fixes30.md. Branch check 2832 passed, 7 skipped, campaign 47.

## 2026-10-29: resolved
