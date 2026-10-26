---
created-at: 2026-10-25
status: open
kind: feature
priority: 95
summary: "about 1000 lines of test scaffolding are copied across test files and crates"
tags: [quality]
---

# Feature: one test-support module

## Report

Copied per file: silence_fatal_panics x5, run_process x5, quiet x6, options() x12 (support/mod.rs already has one), crash_loop x6, the heap and vacuum prelude x3, the running-example schema about 9 times, the rng step 5 times, a private SplitMix64 in sim/tests/clog.rs. fixture, lookup and SCHEMA are identical in adtb-reference/tests/writes.rs and adtb-sim/tests/write.rs; variants() in plan/tests.rs and sim/tests/exec.rs. Move them to one place (adtb-sim/tests/support or an adtb-testkit dev-dependency crate), drop the five allow(dead_code) on shared modules, and put the seed into the assertion messages of the seed loops in faults.rs, latches.rs and clog.rs. Add .config/nextest.toml with slow-timeout = { period = "30s", terminate-after = 4 } and a quick profile with fewer seeds for iteration; the default profile keeps the full sweep. Consider merging the 22 sim test binaries into one.

## 2026-10-25: progress

The nextest profiles landed with lint-ratchet. Still open: the helper moves, and seed sweeps whose coverage thresholds scale with the seed count, so `quick` can cut them (`support::sweep` cuts only sweeps with no such threshold).
