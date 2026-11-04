---
created-at: 2026-11-03
status: open
kind: bug
priority: 560
summary: "a vacuum r throttled by VacuumBudget sleeps on the calling thread, which for a server call is a scheduler worker"
---

# A throttled command holds a scheduler worker

## Report

From background-vacuum V2. `Database::pace` naps on the caller's thread for up to 100 ms per step. Through a server, the caller is a scheduler worker, so a long throttled `vacuum r` holds one worker for its whole run. Fix before the server is exposed: run throttled maintenance commands on the maintenance thread and have the call wait on its completion, or yield the worker between steps.
