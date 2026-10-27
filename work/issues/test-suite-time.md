---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 787
summary: "the default nextest run takes about 4 minutes on 16 cores, with single tests near 2 minutes, and nobody knows whether the tests or the engine are slow"
closed: 2026-10-26
tags:
  - test
  - performance
---

# Feature: Make the test suite fast

## Report

The full `cargo nextest run --workspace` takes 213 to 340 seconds on 16 cores, and agents under load see tests time out at 120 seconds. The slowest tests are sweeps: `adtb-sim::write write_examples_on_several_workers_agree_for_every_seed`, `adtb-sim::pager threads_sharing_a_pager_lose_nothing_across_crashes`, `adtb-testkit::differential rewrites_keep_results` and `adtb-testkit::front_fuzz`. Find out where the time goes: test design (seed counts, repeated setup, debug builds of hot dependencies) or engine and reference cost. Fix both kinds, and set a budget that keeps the default run fast, with the deep sweeps in a separate profile.

## 2026-10-26: landed

Landed in `perf: build tests at opt-level 1, wake one simulated thread per switch, speed up page checksums, diffs and redo, parse built-in sources once, and scale sweeps by profile`. The default run went from 261s to about 20 to 32s at load 23, and summed test time from 3646s to 843s. Sweep sizing now lives in `adtb-testkit/src/profile.rs`; nextest's default profile sets a 20s slow timeout and 60s kill, a `deep` profile runs the full sweeps, and the test-time rule is in `docs/design/runtime.md` 7.7.

## 2026-10-26: resolved
