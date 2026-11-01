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
