---
title: "What partial means"
pick: one
options:
  fault-or-diverge:
    title: "May fault or may not terminate; limits are rejections, never faults"
  fault-only:
    title: "May fault"
---
Raised by the second review of the spec in docs/spec/.

## spec/partiality/fault-or-diverge

May fault or may not terminate; limits are rejections, never faults. An unbounded fix must not move past a guard although no fault is raised.

## spec/partiality/fault-only

May fault. Non-termination escapes the guard rule.
