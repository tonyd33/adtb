---
created-at: 2026-11-05
status: open
kind: bug
priority: 320
summary: "a verify submitted as the first task of a process deadlocks the simulator with every thread parked, on a healthy file"
---

# A first verify task deadlocks the simulator

## Report

## 2026-11-05

Found by the verify-fatal author, present on main. Through a server, a `verify` task submitted as the first task of the process parks every simulated thread; running an `app::users` task first avoids it, an in-process query first does not. The verify-fatal server test runs that task first as a workaround; remove it with the fix. Find whether the hang is the simulator's (a wakeup missed by the runtime) or the engine's (a first-task initialisation waiting on itself); if the engine's, it is a hang a real server can hit.
