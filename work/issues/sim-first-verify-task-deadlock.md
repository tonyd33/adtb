---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 320
summary: "a verify submitted as the first task of a process deadlocks the simulator with every thread parked, on a healthy file"
closed: 2026-11-06
tags:
  - class-failure-flattened
---

# A first verify task deadlocks the simulator

## Report

## 2026-11-05

Found by the verify-fatal author, present on main. Through a server, a `verify` task submitted as the first task of the process parks every simulated thread; running an `app::users` task first avoids it, an in-process query first does not. The verify-fatal server test runs that task first as a workaround; remove it with the fix. Find whether the hang is the simulator's (a wakeup missed by the runtime) or the engine's (a first-task initialisation waiting on itself); if the engine's, it is a hang a real server can hit.

## 2026-11-06: fixed

`fix: wake the background vacuum when a call asks it to yield the vacuum lock`. Cause, engine side: a missed wakeup. A suspended task refused the vacuum lock only set the background vacuum's cancel flag, which the vacuum reads when its pacing timer fires; the vacuum is a daemon and the simulator advances no time for a daemon's timer alone, so every thread parked. A first task met it because a vacuum is due right at open. On a real server: no hang, a stall of up to `NAP` (100 ms) per yield request. Class: class-failure-flattened. What stops the next instance: the lock state is `Option<Holder>`, `Holder::Yields { asked, thread }` keeps the flag with the holding thread, and `Database::try_hold`, the only place that sets the flag, unparks that thread after releasing the lock; `Holder::Keeps` holders read only their call's own cancel token. Regression test, a search: `background_vacuum::a_background_vacuum_gives_way_to_a_first_task_waiting_for_it`, each task kind that takes the vacuum lock (verify, verify one relation, vacuum, set setting, reindex) as a process's first task while the vacuum holds the lock, each case asserting via `vacuum_yields()` that it asked the held vacuum to yield; four of the five deadlock on the parent. The clog_lost workaround is removed. Review found no blockers; its two test findings (reach per case, time over 2 s) fixed. `check --changed` 3469 passed.

Filed from it: verify-restarts-beside-vacuum.

## 2026-11-06: resolved
