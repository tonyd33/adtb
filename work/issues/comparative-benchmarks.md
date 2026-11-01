---
created-at: 2026-10-28
status: in-progress
kind: feature
priority: 910
summary: no reproducible cross-database baseline or recurring measured optimization cycle exists
branch: repeated-execution-cost
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

## 2026-11-01: native canonical-name limitation resolved

The shared naming repair `554f6fba3c75` enables both native canonical-name
regressions. Complete native package now passes 23/23 with zero skips and strict
native lint passes. Exact reports/redaction and before/after/reopened state
remain required; consumers do not strip diagnostic suffixes. This removes the
known naming limitation of the previous schema-control checkpoint.

Known named-set migration admission remains separate. Ambiguous commit,
externally flushed acknowledged-prefix/child-kill controls, hard containment,
resource reporting, feasibility, pilots and the frozen comparison protocol
still precede measurement readiness. No benchmark result or beta exit is claimed.

## 2026-11-01: acknowledged-prefix crash control assigned

Workspace `sqlite-crash-controls`, base `4757015c5dc0`, proposes a bounded real
child-termination control through the existing native/SQLite adapters. Scope:
`adtb-bench` native source, runtime fixtures and README. Define the externally
observed acknowledgement point, retained prefix, identified in-flight ambiguity,
reached kill handshake, bounded process termination/reaping and independent
complete reopen oracle before implementing. Prefer native process facilities;
no new supervisor framework, backend, dependency, manifest or global containment
change is authorized without an explicit scope ruling.

Start with the deterministic counter-smoke workload. Required sensitivity must
reject a lost acknowledged write, unjustified unacknowledged state and a missed
kill point. Preserve exact histories, outputs, support relations and refusals.
This is consumer crash validation; do not claim actual storage-flush fault
injection or engine-wide failure-class closure. No measurements or pilots.

The author owns complete default/native package checks, strict affected lints,
formatting, house and size checks plus every discovery/containment consumer the
new process path actually affects. Unchanged engine/dependency evidence may be
reused after input comparison. Fresh independent review includes child-process
reachability and the oracle; source-stable review overlaps targeted sensitivity.
Final source/native exits and full input snapshots precede landing. The parallel
reference-fixture track has no shared-source overlap; landing is independent.

Approved protocol: four runtime cuts per peer, after acknowledgement 3 or 8,
and before/after the next real commit with its reply withheld. Parent receives
the committed result and syncs its independent journal before allowing progress.
Only the identified whole in-flight write may be uncertain. A separately bounded
verifier reopens the real database and checks the complete independently expected
snapshot and retained searching history oracle. This is transport ambiguity,
not an injected engine flush failure. Parent filesystem/process syscall liveness
is a precondition; universal wall-clock containment remains outside this checkpoint.

Minimal containment extension approved: the existing workspace heap-cap dev edge,
its package-only lock dependency list and one exact new-parent-test slots-2
override. No new package/version/binary/feature or build script. Child commands
use the existing cap helper, one at a time. Verify actual group/cap reachability
and unchanged old groups/executable set. Retain unchanged global discovery proof;
run bounded static slot checks, explicit feature/default binary cap checks and
all complete affected package/lint checks. Do not rebuild every unchanged engine
binary merely to rediscover its existing allocator. Source budget is about
600 test-only helper lines, with no generalized supervisor or IPC framework.

## 2026-11-01: external acknowledgement crash checkpoint landed

Landed `c9cc0be49640`. Both real-file adapters now execute four runtime cut
controls: acknowledged prefixes of 3 and 8 calls, plus the identified next call
before invocation and after real commit with its response withheld. Parent
receives and syncs the acknowledgement journal before permitting progress.
Eight writer kills and eight independent reopen verifications retain all 34
acknowledged results. Reopen checks independently specified complete snapshots,
including empty support relations, and the existing searching history oracle.
Only the identified whole pending write may account for uncertain state.

All database work stays in sequential owned writer/verifier children, using the
existing capped-child helper and one exact two-slot override. Socket and child
exit coordination use one absolute deadline. Parent filesystem/process syscall
liveness remains a precondition; this is not universal hard wall-clock containment
or an injected engine flush failure. Eight actual-source failure probes detect
lost/extra writes, wrong replies, missing kill gates, normal early exit and a
verifier exiting without its validation receipt. Exact source restoration passed.

One final complete default/native package scope passed: empty default with
explicit no-tests acceptance, native 26/26 across two binaries, zero skips,
14.725s execution. Both strict lints, formatting, size and static slot checks
passed. Default/native graph and discovery proof preserve all existing targets,
26 discovery/allocator inputs and old test groups; three actual binary cap
checks abort as expected and both new parents hold the required slots.
All 3340 input records/revision/status and later full filesystem readbacks stayed
exact. Fresh independent source/containment/machine review accepted. Checked and
landed revisions are identical; no after-land package or blanket engine rerun.

House-check returns 1 solely for the reviewed test-only host-clock expectation.
Process deadlines require a monotonic clock independent of an engine I/O runtime;
the narrow exception states its removal condition. This is an accepted review
exception, not a passing command. The initial lint failure, wrong-group failure,
empty-default command exit 4 and probe-script expectation error remain in the
receipt. An earlier house-pass report incorrectly used a trailing print's status;
the final matrix records actual native exits instead.

The feature remains open. Universal hard containment, resource/latency reporting,
large-data feasibility, excluded pilots and an approved frozen comparison protocol
still precede measurements. No benchmark ranking, optimization or beta exit is
claimed by these crash controls. Engine-wide fault/admission/budget mechanisms
and named-set migration admission remain separate.

## 2026-11-01: operational command boundary assigned

Workspace `sqlite-run-boundary`, base `4756caf610e0`, owns the next bounded
comparison prerequisite: practical containment of existing CLI correctness runs.
Compare available native OS process/resource controls before proposing custom
orchestration. Name deadline/stop/kill/reap and descendant ownership, retained
logs/native exit, resource limits and incomplete-state handling. No source or
build starts until the lead rules on the proposed mechanism and exact scope.
Candidate scope is the native tool/README and one narrow run entry if warranted;
no shared engine/runtime, dependency, manifest or global configuration changes
without a concrete ruling. No measurements, new protocol counts or large runs.

The buffered CLI ledger is not externally retained crash evidence. A killed
command with unavailable prefix data must be marked incomplete/invalid, never
silently validated from arbitrary final state or counted as throughput. Retain
partial files and logs; never resume a killed writer as a fresh run. Parent
filesystem/process syscall liveness cannot become an unconditional OS guarantee.
Meaningful sensitivity must exercise the actual boundary and refusal/result
handling. Final affected checks and any new script tests are owned by the author,
with fresh source review and exact receipt reuse for unchanged components.
Reference-fixture work is independent; measurement windows remain exclusive.

Approved scope: native user transient service via one narrow shell launcher,
caller-supplied positive seconds/memory-bytes/task budgets, and opt-in native
facility fixtures plus README. No Rust supervisor or mandatory global systemd
dependency. The manager owns service/descendant lifecycle and resource limits.
Pre-exec actual-cgroup evidence and native ExecStopPost result/exit/InvocationID
receipts must agree; unloaded-unit default fields cannot establish success.
Only successful service result and successful native payload exit are complete.
Timeout/OOM/failure retains partial files and invalidates unavailable prefixes.

One targeted real non-test CLI build is required for actual peer smoke checks.
Keep prior Rust package/lint/graph/discovery receipts after exact unchanged-input
proof; new shell/facility checks certify the changed boundary. No broad engine
or unchanged 26-test repetition. Source-stable fresh review overlaps meaningful
negative probes. Parent manager/filesystem/kernel syscall liveness remains an
explicit precondition. Test fixtures use native facilities, remain opt-in and
fail clearly if their required host support is absent rather than skipping.

## 2026-11-01: native command containment landed

Landed `3dfc772a01a8`. `work/tools/bench-run` executes an existing built native
comparison CLI in a uniquely owned user transient service. Caller supplies time,
memory and task budgets. The native manager enforces service/descendant
lifecycle and resource limits; preexec verifies actual cgroup settings and native
stop-post receipts retain result, exit and matching invocation identity. Success
requires both native service success and normal payload exit 0. Failed, timed-out
or OOM runs retain partial files and mark buffered acknowledgement data unavailable.
Existing run roots are refused; a killed run is never retried over its old state.

The documented opt-in facility command passed from a fresh exact-source layout:
ten runtime cases, three precise preexec resource refusals, both real peers'
complete/reopened state checks and both observed-executable forced timeouts.
Six semantic weakening probes fail at their intended boundaries. One additional
mkdir diagnostic only proves an exact admission status, not harmful retry coverage.
The cleanup witness kills an actual foreground descendant absent from the fixture
PID list and verifies native cgroup absence/member reaping in under one second
against a 60-second sleep. The initial failed cleanup and expiry-only observation
remain explicit, rather than being counted as prompt cleanup success.

All affected syntax, shellcheck, exact-range house and size checks pass. Fresh
source/machine review accepted the six paths; launcher code is 110 shell lines.
One necessary non-test CLI build took 31.607s; native facility execution took
about six seconds. No Rust source, dependency, feature or configuration changed.
Forty-nine unchanged Rust/build/corpus/config inputs and actual binary identity
support reuse of prior package/lint/graph/discovery checks; no unchanged 26-test
or whole-engine rerun was added. All six reviewed content/mode hashes survived
landing; inheritance was only published reference tests and read lead/ticket prose.

Input provenance is scoped: prelaunch immutable source and five actual copied
runtime files were recorded; the full filesystem map was captured after launch.
Later immutable/source, binary/tool and filesystem closure proofs are accepted,
without describing that map as an observed prelaunch snapshot. The unchanged
positive receipts and focused cleanup correction/witness are distinct scopes.

Operational process/resource containment is complete for the documented supported
Linux user-manager environment. Native manager/kernel/filesystem liveness remains
a precondition, not a universal wall-clock guarantee or a reason for another
supervisor. Resource and latency reporting, finite feasibility/pilots and the
approved frozen comparison protocol remain before measurements. No ranking,
optimization, streaming-ACK recovery, engine fault-class closure or beta exit is
claimed. These limits do not reopen the completed containment prerequisite.

## 2026-11-01: complete benchmark reporting assigned

Active user goal: finish benchmark reporting. Workspace `benchmark-reporting`,
base `1860b968a0b8`, owns the complete executable reporting feature, with an
independent read-only acceptance/native-source assessment before scope ruling.
Native command containment remains delivered. Candidate implementation stays
in the isolated comparison tool, runtime fixtures, existing launcher and a narrow
report entry if justified; no engine/shared-runtime or global configuration edits.

Completion requires actual machine-readable raw data and a readable report for
both peers and existing keyed/campaign workloads. Report completed logical units
and throughput; raw end-to-end operation latency with p50/p95/p99 including waits,
retries/backoff and decoded committed completion; attempts/retries/refusals/aborts
with precise denominators; CPU, memory, I/O and storage footprint with native
source/units/scope; and setup/loading/preparation/open/reopen/validation costs
separate from steady operations. Preserve workload counts, schemas, constraints,
durability and correctness/history/reopen checks. Unknown native retry accounting
and genuinely unavailable counters need explicit reasons, never false zeros.

Version/context/config/budgets and completion validity must accompany artifacts.
Incomplete/failed/timed-out/OOM/missing/corrupt runs cannot become successful
comparative throughput. Raw samples must support independent recomputation.
Synthetic runtime fixtures and meaningful actual mutations check aggregates,
units, quantiles, retry-inclusive windows and invalid-result handling; real tiny
both-peer counter/lifecycle reports prove the full path reaches consumers.
Fresh independent source review precedes one final complete affected package,
strict lints/static/shell/reporting consumers, with one author owning native
exits and exact input evidence. Retain unaffected engine checks after bounded
isolation proof; no blanket binary-discovery or engine-suite repetition.

This goal completes reporting capability and its diagnostic verification.
Finite feasibility, frozen protocol and ranked baseline publication remain
subsequent comparison stages; diagnostic outputs must not claim those results.
Machine-specific execution details and raw local diagnostics stay in local
receipts. The report source, portable contract and reproducible commands are
public. Measurement windows remain exclusive of other builds/tests.

Reporting implementation is split by owned interface. `benchmark-reporting`
owns native Rust samples/phases/storage/build identity, canonical version-1 raw
schema and producer fixtures, plus approved native-only serde_json 1.0.154 and
its required private lock/policy additions. `benchmark-report-consumer` owns
native resource capture, the Python standard-library report entry, consumer
fixtures and reporting documentation. The collector/consumer can land first;
the producer owner integrates once and owns final combined package checks and
actual both-peer counter/lifecycle reports. No old-binary final followed by a
repeat merely to integrate the new producer.

Metric rules: preserve finite per-actor closed-loop offering. Latency begins
before first attempt and includes all waits/backoff through terminal decoded
outcome; the steady window is release to last terminal call. Future calls are
not offered at release. Nearest-rank quantiles use ceil(p*N). API calls, committed
write calls and completed four-call lifecycle workflows are explicit units.
Whole-service native CPU/charged-memory resources include setup/oracles/helper;
phase wall spans remain separate. Storage inventories report logical and allocated
bytes at named active and closed boundaries, excluding ledger/report/log files.

Actual collector probes show CPU/memory available, while io.stat is unavailable
on the tested service despite explicit I/O accounting. Retain that reason and
collect supported /proc/self/io process/threadgroup storage and syscall-logical
bytes before artifact writing, with narrower scope clearly labeled. Do not
reconfigure global controllers or invent aggregate/physical-media counters.

## 2026-11-01: collector and report consumer landed

The native resource collector and standard-library Python `bench-report` entry
landed as `9c4de3b56b68`. Independent source and component verification accepted
48 complete/invalid entry cases, two maximum supported workload artifacts, two
native collector outcomes and applicable static checks. All 49 reviewed path
contents and modes survived landing; the inherited delta was only reviewed
assignment-ticket prose. Those component checks were retained without repetition.

The native Rust producer remains unlanded and under independent source review.
Its author integrates this consumer once and owns final combined affected-package
checks, dependency isolation/policy evidence, and actual both-peer counter and
lifecycle reports. Reporting completion, comparison readiness, ranked results and
beta exit are not claimed by this component landing.

## 2026-11-01: complete benchmark reporting delivered

The native producer landed as `64fadee82e3d`, following collector/consumer
`9c4de3b56b68`. The documented `bench-run` then `bench-report` entry now produces
raw call/attempt/workflow samples, versioned JSON and a readable report with
counts, retry-inclusive latency percentiles, named throughput units, native
resources, process I/O fallback, storage inventories, phases and build/context
evidence. Incomplete, failed or inconsistent evidence suppresses comparative rates.
See the [reporting contract](../../crates/adtb-bench/reporting.md) and
[runnable recipe](../../crates/adtb-bench/README.md).

Four actual contained diagnostics passed: adtb and SQLite, each on counter-smoke
and lifecycle with N=4 and four writers. Raw independent recomputation confirmed
48 completed calls, 32 write commits and eight completed four-call workflows,
nearest-rank quantiles, rates, resources, file sizes and phase durations. One
actual counter retry remains inside its call span. All native and report entries
exited zero. These tiny runs establish reporting integration, not a peer ranking.
Native I/O counters remain unavailable on the observed host; the report preserves
that reason and distinctly labels the supported process/threadgroup fallback.

Fresh source and machine reviews accepted the producer and isolated dependency
graph. The complete native package passed 28 tests with no skips; default mode
explicitly accepts its empty test set. Strict affected lints, dependency policy,
formatting and size checks passed. House-check has one explicitly approved
native host-clock expectation, recorded as an exception rather than a pass.
Four actual producer mutants and the retained consumer negative/max/native
fixtures establish their scoped sensitivity. No unchanged engine suite was rerun.

The final checked source was `05354a5c9e5e`; all 68 reporting publication file
contents and modes survived landing. The sole integration delta was the accepted
private Markdown reader contract, which has no build or reporting consumer.
Raw local receipts and host context remain outside the public repository.

The reporting goal is complete. This umbrella returns to open for finite
large-data feasibility, equivalent-outcome protocol freeze, repeated baseline
measurements and publication, followed by one measured optimization. Reporting
does not complete those stages or the embedded beta milestone.

## 2026-11-01: bounded feasibility and protocol freeze assigned

Current user goal: run bounded feasibility, settle common dataset sizes and
concurrency, then freeze the comparison protocol. Base `315af6492c80` already
delivers reporting and containment. Workspace `benchmark-feasibility` owns actual
native feasibility and evidence; `comparison-protocol` owns portable methodology
and a focused README link. Lead decides common parameters and adoption after
independent reading review. No ranked baseline or optimization is assigned here.

Start with candidate N=1000/10000, writers=1/4, both peers, keyed counter with
20,000 requests and lifecycle with N four-call workflows. These are candidates,
not frozen values or successful runs. Establish one release-profile binary from
unchanged source, using locked dependencies; a 300-second build ceiling is
separate from database feasibility. Reuse existing checks for unchanged source.

Run the N=1000 serial rung first, then expand feasible cells toward the complete
16-cell peer/workload/size/concurrency matrix. Each fresh database command uses
`work/tools/bench-run 30 1610612736 64 BIN PEER WORKLOAD RUN N WRITERS`, then
`work/tools/bench-report RUN`, including failures. Execute serially in an exclusive
window. Maximum initial exposure is 16 times 30 seconds plus four 15-second
fallback probes before another lead decision. If a rung fails, retain exact
native reason and partial evidence; do not retry over its directory, extend its
deadline, silently change per-peer work or substitute eight-call smoke work.
Any count/size adjustment must be common and explicit before ranked runs.

Completion requires actual bounded outcomes and resource/phase headroom evidence,
settled meaningful common sizes and concurrency, and a tracked executable protocol
with exact workloads/counts, profile/source/configuration, cache/warmup semantics,
paired order, repetitions and whole-matrix reservation, invalidation/retention
rules, raw reporting and comparison limits. Reconcile older proposals against
the actual runner rather than promising missing warmup/cloning/resource features.
Independent review and lead adoption precede freeze. Host-specific raw evidence
stays local; the protocol and resumable acceptance state are portable.

## 2026-11-01: feasibility ceiling calibration

The unchanged release binary built successfully within its 300-second bound.
Both peers' N=1000, one-writer counter runs reached the 30-second native deadline;
neither produced a complete application artifact. Counts and phase attribution
for those killed runs are unknown. Both peers completed the corresponding
N=1000 lifecycle. The failures remain retained pre-freeze observations.

Before reducing logical work or changing the CLI, the lead explicitly authorizes
new feasibility invocations at a common 120-second native whole-command ceiling,
with the same 1610612736-byte memory and 64-task limits, source and binary.
Calibrate both peers' N=1000 serial counter first and use that allowance for
unlaunched larger lifecycle cells. Do not interrupt a live command or repeat a
successful cell solely to enlarge its allowance. Stop equivalent expansions if
a 120-second rung also fails. No code, durability, constraints, count or per-peer
adaptation is authorized. These are feasibility observations, not replacements
inside a ranked sample set.

This supersedes the initial exposure allocation: at most 20 total database
invocations on this track, each at most 120 seconds, or 40 minutes maximum active
service exposure before another lead decision. If the complete candidate matrix
fits, the prospective common 120-second, 80-invocation protocol has 9776 seconds
of configured runtime/start/stop exposure. Reserve a 180-minute exclusive window
with a prelaunch cutoff and retained not-started cells; native liveness limits
and external reporting/operator costs remain explicit. Parameters are still
pending final feasibility and freeze, not accepted on extrapolation.

## 2026-11-01: intermediate size with a runtime validity gate

Both serial N=1000 counter calibrations completed the unchanged 20,000 writes
under 120 seconds, with exact outputs/history/full-state/reopen checks. Both
N=1000 lifecycle concurrency levels completed. The N=10000 four-writer adtb
lifecycle completed, but its serial counterpart reached the 120-second ceiling
without a final application artifact. Its phase attribution and partial counts
remain unknown. These observations do not support freezing the full 1000/10000
matrix. Freezing only N=1000 would drop the requested dataset-size dimension.

The lead authorizes a narrow runtime-size admission repair before further probes.
The current literal size list forces a Rust rebuild for intermediate parameter
choices even though the generator and oracles support them. Admit N at least
four, divisible by four, dividing 20,000 and allowing at least two complete
counter passes. These rules preserve equal campaign partitions, uniform
closed-form counter expectations and rotated key ownership. Coprimality with
the permutation stride 37 follows from dividing 20,000. This is a bounded
benchmark coverage policy, not a claimed engine capacity limit.

Only leaf CLI/report admission, the portable artifact/usage contract and runtime
boundary cases may change. Do not alter generator, queues, counts, settings,
oracles, durability or engine code. Independent focused source review precedes
one affected package/consumer verification and incremental release build. Reuse
old successful N=1000 feasibility only after proving the gate-only successful
execution path is unchanged, with old and new binary identities explicit.

The next common large candidate is N=5000: fivefold cardinality relative to 1000
and four complete counter passes. Probe the serial adtb lifecycle first under
the same 120-second/1610612736-byte/64-task limits. If it completes, cover both
peers, workloads and concurrency levels plus the missing N=1000 four-writer
counter pair. If it fails, retain it and permit one predeclared N=2000 serial
discriminator before another finite lead decision. At most 11 further database
invocations are assigned in this stage. No new deadline increase, per-peer
adaptation or smoke substitution is authorized.

Land the admission change first, then rebase and finalize the protocol on it.
The protocol keeps two meaningful common sizes, five paired repetitions and
all failure evidence; its final numerical freeze remains pending actual results.

## 2026-11-01: feasible matrix and comparison protocol frozen

The [executable protocol](../../crates/adtb-bench/protocol.md) is frozen in
`c798567c73e8`, with code pinned to `2f0369f68750`. Common dataset sizes are
N=1000 and 5000, concurrency one and four writers, both adtb and SQLite, and
both keyed counter and lifecycle workloads. Counter work stays 20,000 writes;
lifecycle stays N four-call workflows. No durability, constraint, queue,
generator, settings or oracle was weakened to fit the matrix.

All 16 selected feasibility cells completed exact output/history/full-state/
reopen validation: 256,000 calls, 208,000 write commits, 24,000 workflows and
45 outer retry attempts, with no failed or uncertain selected calls. Six old
N=1000 successes are explicitly reused through reviewed gate-only path parity;
ten new executions use the updated binary. This is feasibility evidence, not
a mixed-binary ranked comparison. The five earlier invalid/time-limited runs
and nonselected 10000/four-writer success remain retained. Missing killed
manifests still have unknown partial counts and phases.

Maximum selected caller span was 60.492 seconds under the adopted 120-second
native ceiling; maximum charged peak was 71,962,624 bytes. Adopt 1610612736
memory bytes, zero swap and 64 tasks for every future invocation. The future
baseline has five paired repetitions, 80 invocations, 9776 seconds maximum
configured runtime/start/stop exposure, a 180-minute exclusive reservation and
175-minute latest-launch cutoff. Use one final source and binary per session,
with each cell's configuration fixed by the protocol. Preserve failed and
not-started rows; no automatic replacement or hidden favorable aggregate is
permitted.

The runtime size admission rule replaces literal choices with the generator's
bounded coverage contract. Fresh source/machine review, complete affected package
(29 native tests), 55 exact report cases, six healthy size artifacts, strict
affected checks and source/BIN closure passed. The ten source inputs survived
landing unchanged; only reviewed ticket prose was inherited. Protocol landing
matched its checked tree exactly. Its five synthetic scheduling cases and a
guard-removal mutation verify order, cutoff, failure pauses and preservation of
an existing session. No unchanged engine or native-facility suite was rerun.

Bounded feasibility, size/concurrency selection and protocol freeze are complete.
This umbrella returns to open for the separately scheduled 80-run ranked
collection, independent raw-result review and publication, then one measured
optimization. No ranked baseline, beta exit or defect-class retirement is claimed.

## 2026-11-01: ranked collection, publication and measured optimization assigned

The active user goal is to publish the first SQLite comparison in an exclusive
window, then profile the largest measured bottleneck and deliver a principled
optimization with repeatable before/after results. Completing collection or
publication alone does not complete this goal.

Workspace `sqlite-comparison` collects the frozen 80 new invocations on code
`2f0369f6875073286513ae150e94d113a38f25b7` and one provenance-verified release
binary under the existing protocol. No feasibility data enters ranked results.
Hold the shared machine lock and exclusive window, retain source/binary/config
and per-pair context, stop on nonzero and preserve every planned outcome. No
build, test or profiler runs alongside collection. Other assigned tracks may
read sources and prepare publication/profiling metadata only.

Publish all five per-cell observations and median/range, latency populations,
counts/retries, resource/storage/phase scopes and failures with an independent
raw-data/membership/arithmetic review. Provide portable per-run data and a
reviewed allowlisted raw-sample archive, preserving original-to-public hashes.
Raw host paths, inventories and other private context remain local. Use a
standard repository result page and release attachment after concrete artifact
review; no replacement benchmark framework or hidden favorable aggregate.

Only after baseline publication, select the largest measured adtb bottleneck
using phase/resource and native profile evidence. Bound diagnostic profiling,
keep it outside ranked data, and state the causal hypothesis, invariant and
enforcer before implementation. Compare the strongest alternative and fix the
owning subsystem rather than weaken work, durability, constraints or settings.
No kernel/global configuration change is authorized merely to enable profiling.

The optimization requires an independently reviewed bounded source change,
meaningful regression/sensitivity evidence, appropriate final verification and
repeatable before/after timing on unchanged work/configuration. Shared engine
changes get one full check after review. Preserve the original baseline; record
optimized code/build identity and an explicit comparison revision. Repeat the
affected workload and comparison scope needed to demonstrate the saving and
exclude material regressions. Reject an unrepeatable or noise-sized improvement.
Land and publish the principled change and its results only when these gates
are met. The broader beta remains outside this task's completion claim.

## 2026-11-01: keep comparison data local

The maintainer clarified that no public release, upload or public result page
is needed; the data is for project work. Continue the frozen 80-run exclusive
collection unchanged. Deliver retained raw measurements, a canonical local
machine-readable summary and readable analysis with an independent raw-data
audit. The public artifact/archive/sanitization plan above is superseded.

Profiling can begin after baseline closure, the independent local audit and lead
selection of the largest measured adtb bottleneck. It does not wait for public
publication. The principled optimization and repeatable before/after requirements
remain fully active. Benchmark data and host context stay local; normal code
review, verification and landing still apply to the implementation change.

## 2026-11-01: local baseline accepted; diagnostic profiling active

The frozen exclusive comparison completed every planned invocation with normal
native and report exits, no replacements and unchanged source, executable,
configuration and preparation. Separate author arithmetic and a fresh independent
raw-data audit agree on membership, exact decoded outputs and counter history,
counts, all latency populations, rates, resources, storage and phase boundaries.
Original producer full-state and reopen gates remain distinct from independently
recomputed ledger observations. Retained cache, isolation and unavailable native
I/O limitations remain explicit. Results and raw context stay local, with no
public release or results page.

Workspace `profile-preparation` now owns native diagnostic profiling. The lead
selected the largest measured adtb phase/cell: the steady counter workload at
N=1000 with one writer. An initial CPU profile and unchanged diagnostic control
passed all original workload, output and reopen checks. Caller attribution was
insufficient, so one separate release-equivalent build with symbols and frame
pointers is authorized for attribution. Preserve the exact normal-release before
executable and its source/build inputs; the diagnostic build cannot supply a
ranked performance comparison. No engine change or optimization is implemented.

Next, use actual profile evidence to choose the owning mechanism and its strongest
competing explanation. Require an independently reviewed principled implementation,
appropriate final checks and repeatable normal-release before/after measurements
under unchanged work, constraints, durability and settings. The full active goal
remains incomplete until that optimization and its measured saving are delivered.

## 2026-11-01: bounded leaf range implementation assigned

Native diagnostics distinguish substantial durability-associated waits from
avoidable CPU work. There is no established redundant-force mechanism; Local
acknowledgement remains unchanged. The selected removable cause is shared B-tree
range materialization: the cursor expands every entry of a reached leaf before
testing its bounds, including joined postings it then discards. Workspace
`bounded-leaf-range` implements selection before materialization in that provider.
Preserve exact ordered bound membership, postings, high-key resume, optimistic
validation/retry and required checked malformed-page errors. Add a searching
external-oracle range property and sensitivity to eager out-of-range expansion.
No format, dependency, workload, constraint or durability change is authorized.

Before candidate timing, fix the primary endpoint to counter N=1000/writers=1
steady wall time. Require five new adjacent normal-release old/new pairs in every
one of the eight adtb configurations, unchanged bounds/preparation/oracles, with
all five primary pairs improving and median paired saving at least 5%. Other
cells receive explicit throughput, latency and resource regression review. The
full unpaired baseline spread is not a paired null distribution; the five new
balanced pairs supply finite repeatability evidence without claiming certainty.
Do not lower the floor, change endpoint or extend samples after observing timing.

After independent stable-source review and targeted tests, allow a rejection-only
pilot of two new primary old/new pairs in opposite order. Both must improve and
median paired saving must reach the same 5% floor. Preserve every outcome; a
failed/noisy pilot stops for lead assessment. Pilot points never enter the final
five. Only after pilot admission run the one final shared-engine check and
explicit native benchmark-package verification, then collect the entire new
80-invocation matrix with separate immutable source/build identities and a fresh
independent raw audit. A passed pilot alone cannot justify landing the change.

## 2026-11-01: interference qualification and bounded recovery

The source candidate passed independent review, its targeted sensitivity checks,
one full shared-engine check and explicit native benchmark-package verification.
The confirmation experiment remains incomplete. Verified external foreground
system-configuration evaluation overlapped both members of one pair. A review
of every invoked native interval selected that pair without using rates. Preserve
both original outcomes and raw evidence, including the timeout with no comparable
rate or validated acknowledgement prefix. Overlap does not establish its cause
or a new engine defect.

The frozen proposal explicitly permits a separately ruled replacement for a
verified external interruption; the generated short policy instead said no
replacement without spelling out that exception. Preserve that wording conflict.
The lead adopts the specific frozen exception over the operational default, with
independent methodological review: one linked replacement for each affected
observation, in the original pair order, followed by the original unstarted
sequence. No other observations are excluded and no favourable retry is allowed.
All workload, preparation, durability, native bounds and fixed landing gates stay
unchanged. The accepted logical matrix still requires every original cell and
five paired observations; the extra physical attempts remain visible.

Recovery requires explicit machine-quiet readiness. A process-name guard is only
a partial detector, so resource coordination covers external evaluation and build
work as well as this repository's tests. Use a 30-minute quiet window with latest
new-pair launch at 25 minutes; keep pairs adjacent and preserve any remaining
fixed sequence for another window if that resource cutoff is reached. This is
not a guarantee that every remaining invocation fits, nor a rate-based stopping
rule. Independent per-run audit of immutable unaffected data may proceed while
performance execution is held. Final joined-data audit and all performance and
regression gates remain prerequisites to landing; no new source check is needed
merely because bookkeeping or private analysis helpers change.

## 2026-11-01: complete confirmation rejects bounded leaf candidate

The qualified recovery closed all remaining observations under the unchanged
protocol and native bounds. Separate author computation and independent raw-data
audit agree on all 80 accepted logical observations and the retained 82 physical
slots. All new native executions, reports, source and closure guards passed. The
previously qualified original success and timeout remain preserved separately.

The bounded leaf candidate's primary paired savings were 4.845%, -4.840%,
7.777%, 15.851% and 7.106%, with median 7.106%. The fixed landing rule requires
every primary pair to improve. Four of five does not pass, so the candidate
remains unlanded despite its positive median and successful correctness checks.
Other steady-state gates did not reject; isolated latency, resource and storage
review items remain explicit. No extra sample, exclusion, threshold change or
repeat of the same candidate is authorized to obtain a passing result.

The local SQLite baseline and diagnostic profile remain accepted. The requested
optimization is still undelivered. Workspace `repeated-execution-cost` now owns
a bounded source investigation of the other measured CPU providers: repeated
execution compilation and store metadata construction. Establish actual steady
caller reach, ownership and invalidation before choosing one structural change.
No implementation or new measurement is authorized by that investigation alone.
A new candidate needs its own reviewed mechanism, sensitivity evidence, final
verification and predeclared complete normal-release comparison. Failed candidate
observations cannot validate it. Work, guarantees and durability remain fixed.

The next selected mechanism shares database-owned immutable heap/index metadata
across executor views, while constructing fresh snapshot, SSI reader and history
state per view. Scope includes changing the low-level executor Store map fields
to shared immutable maps and updating their Rust construction sites. Keep owned
setup bindings and observable database APIs unchanged. This representation change
does not introduce cross-owner caches or new identity/invalidation rules.

Select a new combined candidate with the already reviewed bounded leaf provider
before implementation or timing. Both mechanisms remove avoidable copying at
their owning boundaries; their individual savings are not assumed additive.
Validate the combined tree with independent review, searching ownership/view
oracles, meaningful mutants and one final shared-engine check. Use one new
rejection-only two-pair pilot in opposite orders, then 80 entirely new normal-
release observations if admitted. Preserve the original workload, limits, primary
endpoint, all-five improvement rule, 5% floor and every regression guard. Pilot
points and all rejected candidate points stay outside the new final matrix.
