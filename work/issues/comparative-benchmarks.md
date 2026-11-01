---
created-at: 2026-10-28
status: in-progress
kind: feature
priority: 910
summary: no reproducible cross-database baseline or recurring measured optimization cycle exists
tags:
  - engine
  - test
---

# Feature: Comparative benchmarks

## Report

The plan mentions SQLite benchmarks at embedded beta, but no concrete track
defines readiness, comparable workloads, reproducible results or optimization
rounds. Existing simulator timings and subsystem performance tickets do not
establish comparative performance through normal application APIs.

The user requested cross-database benchmarks and repeated optimization rounds
on the roadmap, leaving timing to the lead. This is planned work, not a claim
that measurements exist or that adtb is faster.

## 2026-10-28: scheduling and acceptance

The lead schedules an initial embedded comparison after the current authority,
permission and checked-binding correctness work is verified and the campaign
concurrency/reopen workload is reliable. Run before broader server expansion;
do not wait for all operations features or deferred version translation. This
does not displace an active engine correctness repair. Reevaluate readiness at
each correctness milestone checkpoint. The scheduling gate is a verified slice,
not completion of every item on the broad milestone/roles tickets, so those
tickets are not artificial whole-feature dependencies here.

SQLite is the first embedded baseline under project/workload/embedded-oltp.
Research additional peers, maintained versions and available benchmark tools
at kickoff, following the KB workflow. Do not choose a new harness or dependency
without comparing existing tools and native interfaces. Dispatch implementation
to fresh agents in isolated jj workspaces with bounded briefs and landing order.

Acceptance for the first comparison:

- Versioned deterministic workloads and data generators; repeatable commands
  using actual application APIs and real runtime/storage. Document revisions,
  peer versions, build flags, hardware, filesystem, memory budgets, cache state,
  warmup and repeated-run procedure. Preserve raw measurements and variation.
- Ordinary OLTP controls plus campaign/ADT workloads covering constructor
  selection, indexed relationships, joins, atomic writes/retags and enforced
  invariants. Cover dataset sizes and concurrency levels. Measure open/reopen,
  loading/preparation and evolution separately from steady-state operations.
- Verify equivalent logical outcomes, isolation, acknowledged-write durability
  and constraint enforcement on every peer before timing. Include necessary
  schema encodings, triggers or transactional checks in the measured path.
  Document refused/unsupported cases and any remaining mismatch rather than
  silently weakening guarantees or treating omitted work as a speedup.
- Report completed logical transaction throughput, p50/p95/p99 latency, retry
  and abort rates, CPU, memory, I/O and storage footprint. Application latency
  includes queueing and retries. Distinguish throughput under offered load from
  successful work and retained state. Publish losses as well as strengths.
- Freeze the baseline before tuning. Profile measured hotspots, dispatch a
  bounded principled fix to the owner, run correctness checks and compare the
  same workloads before/after. Repeat rounds based on measured impact; reject
  noise, weakened semantics and unacceptable regressions. A correctness failure
  stops tuning and invalidates affected results until repaired and rerun.

Extend to client/server peers after the first authenticated server path has
verified binding, bounded admission/result lifetime and cancellation. Keep
embedded and client/server comparisons separate and expose protocol overhead.
Peer selection happens at that phase; no server readiness is claimed now.

Preserve a reproducible report with workload-specific conclusions and remaining
bottlenecks. Rerun relevant comparisons after major planner/executor/storage
changes and before embedded beta/server releases. Existing engine-open,
planner/executor and heap performance tickets are candidate fixes to prioritize
from profiles, not a mandatory prerequisite list or assumed causes of poor results.
No benchmark, optimization or code change was performed for this roadmap entry.

## 2026-10-30: one agent at a time

Benchmark work runs as one agent at a time, so runs do not contend for
resources. While it measures, no other track builds or tests on the machine;
record `uptime` with each run and discard a run taken under other load.

## 2026-10-31: first baseline selected next

The finite public commit recovery checkpoint and current-pin campaign
authority/binding paths have verified receipts. A source assessment
confirmed real-file generated-call concurrency/reopen paths; simulator load
timing remains correctness evidence only. The first baseline is selected
after the active pinned field-reader repair and fixture checks, with an
exclusive measurement window. Do not wait for every operations feature.

Before implementation, freeze a protocol for one ordinary keyed transaction
control and one current-pin ADT relationship/retag/invariant workload, two
data sizes and serial/bounded concurrent writers. Compare existing native
interfaces and tools before choosing a harness or dependency. Check peer
versions and keep sourced facts through the knowledge-base workflow.
No benchmark result or readiness of every skewed-pin/role/socket path is
claimed. Unsupported operations and differing guarantees invalidate or
limit the affected comparison rather than being counted as fast success.

Keep private execution-environment details in local receipts under the
repository's publication rules. Protocol, logical workload, repeatable
commands, portable configuration and reviewed results remain public.


## 2026-11-01: native correctness driver started

Track `sqlite-baseline` owns the reduced owner/current-pin correctness driver:
a keyed transaction control and Party/Npc plus Planned/Live/Played session
lifecycle. Implementing this supported consumer may proceed alongside the
isolated pinned-reader redesign; no rolling schema, role, hiding or nested
upsert policy is assumed. Validate the actual public schema and calls before
claiming readiness. This supersedes the earlier blanket ordering for source
implementation; timings still require verified prerequisites and review.

Prefer a root workspace member using native Cargo opt-in features so bundled
SQLite and long workloads do not add routine default build/test cost. Compare
existing codegen/native interfaces before committing to the concrete layout.
No new benchmark framework, third workspace or engine API change is assumed.
The proposed rusqlite 0.41.1 bundled route pins libsqlite3-sys 0.39.1/SQLite
3.54.1, subject to actual complete dependency/license/advisory verification.

The finite implementation exit is reviewed deterministic adapters, equivalent
constraints, independent complete and acknowledged-prefix/reopen oracles,
exact refusal controls and a portable run recipe. No timing, speed claim,
optimization, pilot or measured matrix has started. The separate pilot and
exclusive measurement phase follow reviewed correctness evidence; the lead
must freeze counts and time budget before ranked measurements.


## 2026-11-01: correctness configuration and request partition

Native lifecycle checks exposed the engine's aggregate admission rule:
each call reserves 64 MiB, so configure the process query budget as writer
count times 64 MiB for this frozen engine revision. Keep the 64 MiB page
pool and SQLite total page cache separately labelled; equal total query
memory or RSS is not claimed. Serial and four-writer small lifecycle cases
passed exact outputs and full state/reopen after this configuration change.
A failed concurrent 64 MiB run retained its exact acknowledged prefix and
reopened consistently. No engine budget constant was changed.

Before measurement freeze, correct the counter partition to actor(i) =
(i + floor(i / N)) mod writers, retaining the same request-to-key permutation.
The earlier every-fourth-request assignment gave each key to the same actor
because both dataset sizes are divisible by four. Rotating ownership between
passes gives shared-key access across actors with balanced work; assert that
property for both planned sizes. It does not guarantee simultaneous clashes.
Use bounded native SQLite busy waiting with recorded settings; rapid retry
exhaustion under an explicitly zero busy timeout is not peer incapability.
Keep original incomplete run evidence and include waiting in call latency.

The first driver checkpoint may land with explicit remaining full refusal,
constraint-mutation and externally flushed acknowledgement/child-kill
controls. Such a checkpoint is not measurement readiness or ticket closure.
Small correctness checks use fixed small request counts; no repeated long
workloads are required merely to iterate the driver. No ranked measurement,
pilot, operation-count freeze or performance claim has occurred.


## 2026-11-01: isolated consumer verification scope

Fresh review accepted the initial driver architecture, then found that its
unordered counter oracle could accept impossible call order. The correction
uses actor and per-key precedence with bounded placement of identified
uncertain calls. A separate operational interleaving model checks 160000
small histories, including 57600 uncertain histories, and live audit witnesses
reject descending returns and a cross-key cycle. Focused delta review accepted
that correction. The six opt-in package tests pass in under one second;
this does not certify the deferred native crash and constraint controls.

An independent locked graph comparison found every existing package's
resolved features, normal/build/dev dependencies and metadata unchanged in
default and native modes, with all old lock records unchanged. The tool is
an opt-in leaf; its generated build output is owned by that tool. Apply the
isolated-leaf verification rule: final complete native package checks,
default/feature lint evidence, full dependency policy and the affected
workspace heap-cap discovery/manifest/slot consumers. Retain explicit native
binary containment evidence because default discovery does not select those
feature-gated targets. This supersedes earlier full-engine-check wording for
this isolated consumer only. Shared production or unproved global impact
still requires broader checks; prior component receipts do not test new code.

The final owner verifies the combined tree preserves the independently landed
reference fixtures and captures unchanged inputs and command exits. No default
engine/campaign/Loom/Miri rerun is required solely by adding private opt-in
packages. Final scope, review and integration evidence remain necessary before
landing the checkpoint. Measurement readiness and this feature remain open.


## 2026-11-01: first native driver checkpoint landed

Landed `e68af8b5487d` after independent source review and one focused delta
review. The tested combined tree is identical to the initial landed tree.
All 24 reviewed tool/manifest/fixture inputs matched; all 3214 tracked file
hash/mode/symlink entries, commit identity and status stayed unchanged through
final verification. Native package: six tests passed, zero skipped, 0.499s.
The three selected workspace heap-cap consumers passed; binary discovery
and its nested default-workspace build took 374.476s. This is build/discovery
cost, not benchmark timing or semantic engine-test execution. Formatting,
house and size checks passed. Unchanged strict default/native lint,
locked-graph dependency policy and explicit native-binary containment receipts
were retained under the isolated-leaf rule. No duplicate full engine run ran.

The checkpoint supplies real native generated calls and retained SQLite
prepared statements, deterministic complete/prefix/reopen oracles, and the
corrected compatible-history checker. It introduces no comparative timing,
measurement readiness, optimization or beta-exit claim. Full declaration/FK/
subset/unique/transition refusal controls and corresponding mutations, actual
ambiguous-commit and externally flushed child-kill controls, hard containment,
resource reporting and large-data feasibility remain open. Keep this feature
open for those controls and the later reviewed measurement phase.


## 2026-11-01: declaration and refusal controls assigned

Workspace `sqlite-controls`, based on `89891a340f30`, owns a finite runtime
case matrix against the existing real-file native and SQLite adapters. Cover
keys, references and constructor subsets, uniques, constructor shape and the
Played transition, with related positive controls and complete before/after/
reopened state. Keep independently specified typed outcomes and native error
payloads. Cases excluded by generated types or refused during text checking
are distinct from runtime constraint enforcement. One native retag corresponds
to SQLite UPDATE; separate DELETE/INSERT statements do not establish an equal
statement boundary. No transaction-wide transition promise is added.

Scope is the existing leaf tool's native test helpers, runtime fixtures and
README. No new dependency, feature, binary, global runner or engine source.
Keep known canonical-name defects normative and explicitly blocked; never
normalize their output into an equivalence claim. A separately identified
existing-defect regression may use the repository's explicit ignored-test
convention with an actual failing receipt and ticket reference, outside the
passing matrix. Missing or skipped cases cannot count as passing coverage.

Acceptance: independent source review, actual declaration/SQL-constraint
mutation witnesses with exact restoration, complete default/native tool-package
checks and strict affected lints, and applicable static checks on unchanged
tracked inputs. The author owns final verification. Retain unchanged graph,
policy and global-discovery evidence under the isolated-leaf rule. This
checkpoint adds no timing, large-data feasibility, crash-control, hard-watchdog
or measurement-readiness claim; those remain subsequent bounded work.


## 2026-11-01: schema-control checkpoint landed

Landed `6510d123e637` after fresh source and retained-execution review.
Runtime data supplies 31 shared cases, ten SQLite-only tagged-shape cases
and two SQLite unscoped-unique cases: 74 active peer runs. Independent native
outcomes, reports and expected logical state are checked before/after/reopen.
Refused SQLite writes also preserve all columns, including unused encoding
columns. Type exclusion, admission, runtime faults, statement boundaries and
refusal-order differences remain explicit. The existing counter oracle and
normal benchmark execution paths are unchanged; new helpers are test-only.

Fourteen named probes cover 13 distinct declaration/SQL mutations; CHECK removal
was repeated after adding the raw-state oracle. Actual guards were removed,
native schema pins were corrected before semantic runs, and every probe failed
its intended state/outcome assertion. Compound foreign-key removal is labelled;
no single-declaration sensitivity is claimed for implied constraints. All
mutations were exactly restored. The two native canonical-name regressions
were explicitly run and failed for the known scope suffix, with correct report
rows and state; they remain separate ignored tests, not passing coverage.

Complete default package: zero tests. Complete native package: 21 passed,
two explicit known skips, 12.145s execution and 2.28s compilation. Strict
default/native lint, formatting, house and size checks passed. Earlier control
runs took 8.761s and 11.382s; individual families reached 2.973s. Splitting
families is not a speedup, and no cause is inferred from this variance.
These are fixed runtime-corpus enforcement sweeps, not benchmark measurements.

All 3271 tracked inputs/metadata entries and tested id stayed identical; only
the parent's bookmark decoration changed in status, with identical file entries.
All 19 reviewed tool inputs and 38 retained dependency/build/discovery/tool
inputs matched. Landing added only independently certified reference test/data
and reviewed ticket prose; every tool input remained identical. The leaf
boundary justifies receipt reuse without another engine/discovery run. The
checkpoint took approximately 19 minutes from local brief creation to landing;
that is a whole-track observation, not coding time or a general speedup.

Keep the feature open. Canonical naming, ambiguous commit and externally flushed
acknowledged-prefix child-kill controls, hard containment, resource reporting,
large-data feasibility, excluded pilots and frozen measurement protocol remain.
No benchmark result, optimization, beta exit or defect-class closure is claimed.
