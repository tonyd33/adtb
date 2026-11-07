---
created-at: 2026-11-07
status: open
kind: bug
priority: 250
summary: "in the simulator a commit's log force does not switch threads, so a writer loop that never reads starves every other thread and tests depend on incidental reads for interleaving"
tags:
  - class-weak-tests
---

# a simulated log force is no switch point

## Report

Found 2026-11-07 by page-incarnation M1: once an allocation stopped reading the free page, a backup test's writer ran 49331 commits (317 on main) before the backup thread ran, and three reach floors fell because their interleaving came from those reads. A log force is a durable write and a real wait; the simulator should make it a switch point with its latency, so interleaving does not hang on unrelated reads. Changing it reschedules every simulated test: measure the reach floors and timings on 32 seeds before and after.

2026-11-07, periodic-checkpoint change 2: with every page resident, an unpaced load in the 16-page tests ran 8 thresholds with no checkpoint until it slept; the tests pace the load (half a threshold, then 1 s). A force or put that never parks the thread starves background workers in the simulator.
