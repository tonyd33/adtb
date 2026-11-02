---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 900
summary: "embedded beta remains in progress: operations features, resource controls and broader recovery coverage remain"
depends-on:
  - milestone-8-embedded-alpha
  - statistics-and-plan-pinning
  - dump-restore-backup
  - ops-config-resource-metrics
  - comparative-benchmarks
  - command-follow-ups
  - verify-online
  - quarantine-wired
  - scrub-worker
  - background-vacuum
  - compaction
  - reindex-jobs
  - activity-and-metrics
  - real-backend-faults-engine
  - format-settings
tags:
  - milestone
---

# Feature: Milestone 9, operations, embedded beta

## Request

Milestone 9 of docs/plan.md. Exit: the suite and every test method pass; benchmarks are published with the release. Tony, 2026-10-29: it may run out of order, but it must not be skipped. It had no ticket until now. vacuum and verify-depth are resolved. Before dispatching a slice, audit each docs/plan.md bullet against the code and the manual's list of what the engine still refuses.
## Proposal

## Open questions

## 2026-10-29: audit

work/briefs/review-m9-audit.md: 6 of 53 requirements done, 14 partial, 33 absent. Order: verify-online, quarantine-wired, scrub-worker, dump-restore-backup (S4 to S6), then settings and the rest. Rulings adopted so far: spec/verify-structure/structure, spec/verify-scope/per-scope, spec/tool-name/adtb; OpenTelemetry and traces move to milestone 10. The other rulings in the report are made when their slice is dispatched.

## 2026-10-31: next step

By the wide review, new background jobs wait for fault-matrix. Order: test-audit-findings, then fault-matrix, compile-time-identity and vacuity-gate, then the remaining slices named in depends-on. Exit needs a searching test named for each claim, and the audit of 2026-10-29 must be re-run first, since its counts are stale.

## 2026-10-31: finite recovery checkpoint and next increment

The bounded public statement/commit recovery checkpoint landed as
`9e349658d759`, with attempted/acknowledged state, reached fault positions
and three mutation witnesses. It does not close the wider fault matrix or
real-backend coverage. Hidden conflict scope and surviving field identity
are fixed; nested upsert reader facts remain an active bounded repair.

A fresh source assessment covered all six M9 plan bullets, not every one
of the older 53 subrequirements. The old 6/14/33 counts are stale. Verify,
quarantine, scrub, reindex and full physical backup/restore have public
callers; statistics/plan lifecycle, background vacuum, logical dump,
incremental/PITR, metrics/catalog releases and real-backend fault searches
remain partial or absent. None of this is a milestone-exit verdict.

The next selected milestone increment is the first reproducible embedded
SQLite baseline under the existing benchmark exception in docs/plan.md.
Freeze one keyed transactional control and one current-pin ADT relationship,
retag and invariant workload, with two data sizes and serial/bounded
concurrency. Independently establish equivalent outcomes, serializability,
constraints and acknowledged durability before timing. Keep preparation,
open/reopen and steady-state costs separate, retain raw repeated samples,
and report unsuccessful work and unsupported cases explicitly.

Measurement gets an exclusive window after active correctness checks.
Then choose one measured optimization or the bounded background-vacuum
consumer. This supersedes the earlier appended all-umbrella sequencing:
fault-matrix, compile-time-identity, admission and budget tickets are
prerequisites only where the selected consumer actually needs them. They
remain tracked debt and beta exit still requires the published benchmark
and all stated release guarantees. No user-owned design ruling changed.

## 2026-11-01: reporting capability delivered

Native benchmark reporting now has a documented executable entry, raw samples,
human/JSON summaries and complete tiny-run integration on both adtb and SQLite.
Independent recomputation checks actual results; invalid evidence cannot acquire
comparable rates. This completes the reporting slice of comparative-benchmarks,
not the frozen or published comparative baseline.

The pinned-reader correlation and preallocation-budget contract is also accepted
and tracked inside the permission subsystem. Native reader integration remains
unimplemented and its bug ticket stays open. No new refusal policy or beta-exit
claim follows from this contract.

Next comparison steps remain finite feasibility, equivalent outcomes, protocol
freeze and repeated baseline publication before selecting an optimization. The
other operations and correctness obligations above remain unchanged; milestone 9
is still in progress.

## 2026-11-01: comparison protocol frozen after feasibility

All 16 selected adtb/SQLite counter/lifecycle cells passed bounded feasibility
at N=1000/5000 and one/four writers. The executable comparison protocol is
frozen, with unchanged work and correctness guarantees, five paired repetitions
and explicit native limits, reservation, raw evidence and failure rules. Earlier
timeouts remain visible and are not reclassified as successful measurements.

The next comparison increment is the separately scheduled 80-invocation ranked
baseline, followed by independent raw-result review and publication. This closes
the feasibility/protocol slice, not the benchmark umbrella or embedded beta.

## 2026-11-01: initial comparison accepted; return to correctness and operations

The maintainer accepts the first SQLite comparison, profiling and bounded
optimization exploration as complete. Comparative-benchmarks is resolved for
this initial scope; further tuning is deferred. No public release is needed for
this checkpoint and no rejected engine optimization is landed. This is not an
embedded-beta exit. Reassess benchmark freshness if relevant engine behavior
changes before release, rather than extending the completed optimization search.

Next prepare bounded native pinned-reader integration against the accepted
private correlation/budget contract, preserving authority evaluation and exposing
nonhiding admission as a separate unresolved question. In parallel, select one
additional public recovery workload or fault domain using the existing enumerator
and exact acknowledgement oracle. These are planned tracks, not running agents.
Use fixture data by default and one final verification owner per code track.

Refresh the operations exit checklist against current code before selecting the
next consumer, provisionally background vacuum. Statistics/plan lifecycle,
configuration and resource observability, remaining logical dump/incremental/PITR
work and real-backend fault coverage still require bounded slices. Shared
admission and work budgets must precede exposing untrusted socket text. Existing
umbrella tickets do not require unrelated rewrites before every consumer, and
new feature work does not displace a demonstrated data-loss or authority defect.
