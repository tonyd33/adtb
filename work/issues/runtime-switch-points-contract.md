---
created-at: 2026-10-25
status: open
kind: bug
priority: 595
summary: "runtime.md says the simulator switches threads only at runtime calls, but it also switches inside page memory"
tags:
  - quality
  - design
  - sim
---

# runtime.md says threads switch only at runtime calls, but page memory switches too

## Report

`docs/design/runtime.md` §1 states "The simulator switches threads only inside calls to these interfaces", and §7.6 that a crash unwinds every thread "from its switch point" there. Since `pages::switch_points` (`crates/adtb-sys/src/pages.rs:87`) and `Config::switch_rate` (`crates/adtb-sim/src/sim.rs:33`), a simulated thread can also switch, and unwind on a crash, before a latch, an optimistic read, an upgrade or a validation, and halfway through `Optimistic::read`. `docs/design/sys.md` §1 says a scheduler "switches threads only at runtime calls". The engine may now rely on neither statement, and nothing states that page memory calls are switch points the engine must survive unwinding from.

Fix: state in runtime.md and sys.md that page memory calls are switch points when the simulator enables them, and what the engine must honour there (no state left half-updated across them).
