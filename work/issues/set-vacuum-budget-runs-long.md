---
created-at: 2026-11-16
status: open
kind: bug
priority: 250
summary: set setting VacuumBudget on a fresh file ran over two minutes using CPU
tags:
  - engine
  - class-none
---

# Set vacuum budget runs long

## Report

Seen by the review of catalog-encoders part A (2026-11-16) on a build of main at 7d30fd21, not reproduced by the lead: through the shell, `set setting VacuumBudget = { io: 0.2, cpu: 0.5 }` on a freshly created file ran for over two minutes using CPU before it was killed. Setting `RetentionWindow` on the same kind of file returned at once. Cause unknown; related tickets on the budget: vacuum-cpu-measure, throttled-command-holds-worker.

First step: reproduce with a timeout, and read where the statement spends its time (a stack sample) before forming a theory. Decide whether it is a hang, a loop, or a throttle applied to the setting write itself.
