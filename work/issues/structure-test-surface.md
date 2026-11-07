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
