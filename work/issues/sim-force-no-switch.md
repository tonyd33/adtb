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
