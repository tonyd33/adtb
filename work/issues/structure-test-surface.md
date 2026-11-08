---
created-at: 2026-11-07
status: open
kind: feature
priority: 11
summary: tests reach engine internals through write_path at 356 sites and rebuild the harness per file; tests get one stated surface so engine internals can be sealed
---

# Feature: A stated test surface over one harness and one fault mechanism

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-harness.md (local). `write_path()` 356 sites in 23 files, 152 into `.pager`; 27 `doc(hidden)` hooks on `Database`; every layer of adtb `pub mod`; `open` defined 33 times; four fault mechanisms with two `Faults` types that draw differently; simkit's `Hooked` breaks runtime.md §2 (no unpark for injected completions); simkit keeps its own copy of the SSI protocol, so the isolation search never runs `write::Database`; simkit binds a catalog the engine never produces.

Exit: one harness crate surface with a written contract; one fault mechanism; the isolation search runs the engine's write path; `write_path()` and `doc(hidden)` on `Database` at 0 outside that surface.

## 2026-11-07, design and rulings

Design: work/briefs/design-test-surface-r1-report.md (local). An engine-owned `adtb::testing` module behind a `testing` feature, its contract in a new docs/design/testing.md, handing tests values it builds, never `&Pager`, `&Heap`, `&Index` or `&write::Database` (`testing.store()` the one exception until exec is sealed); I/O faults, crashes, reopen, byte damage and the post-crash check in the harness. One `Faults` in `adtb_sys::runtime` with the simulator's draw order; `Hooked` becomes `Sim::plan` in the simulator's completion path; `Op::kind()` replaces name matching; engine plants a `Plant` enum. The isolation search runs `adtb::Database<SimRuntime>` and simkit/engine.rs is deleted. support.rs's binder is replaced by the engine's binding. Counts corrected: `.pager()` from tests 337 in 23 files; `doc(hidden)` in api/mod.rs 13.

Order: T1 to T3 land before storage steps 2, 4 and 5; storage step 1 may go first; T4 to T8 are independent of storage.

Lead rulings on its open questions:
- `testing` unified into a workspace build is accepted; a check fails when a per-package build of the server binary has the feature.
- Moving pager-level tests into `adtb` as unit tests is part of this ticket's exit (step T9), since it is what lets the internal modules stop being `pub mod`.
- The harness's crash loop owns the post-crash check, every crash runs it by construction, and runtime.md §5 says so.
- The report's other recommendations are accepted as written; `archive_status` goes to the spec through the spec-revision workflow when T-steps reach it.

## 2026-11-07, T1 and T2 landed

`adtb::testing` with `Testing`, `Probe` and `TxnProbe` (owned values only), its contract in docs/design/testing.md; every pager and transaction observer moved off `write_path`. Counts: `write_path()` 356 to 258, `.pager()` from tests 337 to 248, `doc(hidden)` in api/mod.rs 13 to 10, pager and transaction observers in tests 213 to 0 (the measure excludes `SimRuntime::recorded`, the simulator's own). `work/tools/feature-check` refuses any normal or build dependency on adtb with `testing`, read from cargo metadata. Next: T3.

## 2026-11-07, T3 landed

Plants, fault controls and knobs go through `adtb::testing` (`Plant` with `Planted`, `Knobs::with_segment_size`); `write_path` deleted. Counts: `write_path` 258 to 0, `.pager()` 248 to 0, `CreateOptions` pub fields 25 to 24. `mark_of` asks the engine's mapping (`relation_mark`, `heap_mark`, `index_mark`); `own_mark` added for tests that plant a mark naming one heap or index, a separate promise in docs/design/testing.md §3.1 (lead ruling: kept, it names a mark's identity and holds no mapping rule). Next: T4 to T8, alternating with storage owners.

## 2026-11-07, T4 landed

One `Faults` in `adtb_sys::runtime` (was two, with different draw orders), with no default: every site states its geometry, fixed for a runtime's life in both backends. Pure decisions per op kind (`decide_write`, `decide_read`, `decide_allocate`, `decide_force`) and for an unforced write at a crash (`decide_unforced`), their draw order stated in runtime.md §3 and checked by sweeps in runtime/tests.rs with a recording draw source. Both backends apply fates and draw nothing for an op themselves; the simulator's draws are unchanged (crash image digests, damage profile). The real backend now answers `Full` under `no_space`, with an io_contract case. Counts: `struct Faults` 2 to 1; pub fields in os/faults.rs, disk.rs and sim.rs 27 to 7, with 17 in the new runtime files. Next: T5.

## 2026-11-07, T5 landed

Planned per-op actions live in the simulator: `sim.plan(Plan)` with `Match` (pure data: kinds, path tests, offset, a rename's source, create; union and difference) and `Action::{Fail, Lose, Run, Hold}`, applied in the one completion path, so every completion unparks its submitter. Observers read a per-op submission log (`logged(Match)`) without the plan lock; there is no closure in `Match`. Rules end with their run unless planned `lasting` (the damage search's torn and unreadable injections, which survive a restart as on main); numbering lasts across a simulation's processes; a search that owns its plan refuses any other. A planned Fail on Close or Lose on a non-write is refused when the plan is built; planned writes still queued at a crash never land. `Op::kind` replaces op names; `Ticket` has a private field and one constructor that clippy's `disallowed_methods` admits only in the two backends. Counts: `Hooked` 129 to 0; runtime impls outside the backends 9 to 3 (the page-memory waiters); forged tickets 1 to 0; string op names 21 to 0. Unplanned runs draw as on main; the damage shard reports are unchanged line for line. Next: T6.
