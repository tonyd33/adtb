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
