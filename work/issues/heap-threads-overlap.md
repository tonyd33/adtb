---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 590
summary: "the heap thread tests pass without checking that their threads overlapped, and a seed loop omits the seed"
branch: adtb-qtest
tags:
  - quality
  - sim
  - test
---

# The heap thread tests can pass with their threads running one after another

## Report

`heap_only_updates_from_many_threads_leave_the_directory_root_unlatched` (`crates/adtb-sim/tests/heap_threads.rs:114`) runs with `Config::default()` (`:116`), so `switch_rate` is 0 and the writers switch only at runtime calls; with a 1024-frame pool they may run back to back, and nothing checks that two writers' updates overlapped. Its seed loop's other assertions omit the seed: `row {key} moved off its page` (`:95`), the page list (`:126`), the verify result and the per-row comparison.

`clearing_bits_while_the_visibility_map_root_grows_keeps_the_map_whole` (`:192`) runs one seed at `switch_rate: 0.3` and has no count showing that a clearer's bit clear ever ran while the grower was mid-growth, so a change to the schedule can make all 40 rounds sequential without the test noticing.

Fix: run the first test with a nonzero `switch_rate` and put the seed in every message; in both, record that the threads interleaved (a clear between the grower's start and end, an update begun while another writer's is in flight) and assert the count.
