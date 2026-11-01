# Executable embedded comparison protocol

Status: frozen after independent source/method review and an audit of all 16
selected feasibility cells. The matrix is two peers (adtb and SQLite), two
workloads (counter and lifecycle), N=1000/5000, writers=1/4 and five paired
repetitions: 16 cells, 80 invocations. The frozen settings and finite reservation
below apply to future ranked collection. No ranked sample set was collected by
this feasibility work; calibration stays outside that set.

## Work and guarantees

Run the unchanged native consumer through generated current-pin adtb calls or
retained rusqlite prepared statements. The migration, bundle and SQLite encoding
are `fixtures/log/0001.adt`, `fixtures/baseline-1.0.adb` and
`fixtures/schema.sql`. Both peers must meet independent expected outputs and
complete relation snapshots, including empty/support relations; agreement only
between peers is insufficient. Preserve active keys, references, constructor
subsets, branch checks, Live uniqueness and Played transition enforcement.
Retain the reviewed refusal/mutation and externally journaled child-kill control
receipts for this exact adapter/schema before freeze. Those controls are separate
from each timed command's output/history/full-state/reopen checks.

N is loaded primary-row cardinality, not operation count:

| Workload | Initial population | Required completed work |
| --- | --- | --- |
| counter | N zero-valued counters; all support/session relations empty | 20,000 atomic read/increment/return write calls; no workflows |
| lifecycle | N Planned sessions, 4 campaigns, 4 Party and 4 Npc entities; counters empty | N workflows, 4N calls, 2N writes and 2N query calls |

The deterministic key order is `1 + ((i * 37) mod N)`. Counter request i belongs
to `(i + floor(i/N)) mod writers`; passes rotate key ownership. Shared keys do
not guarantee simultaneous contention. Lifecycle actors own campaign queues;
four actors each own one campaign, preserving its single Live slot. Each
workflow performs Start, joined Live read, Finish, then keyed constructor-match
Played read. Only that last decoded read completes a workflow.

These are finite closed-loop actors with one call in flight each. Calls are
offered after the previous call completes; future queue entries are not offered
at the release barrier. This measures bounded offered concurrency, not an
open-loop arrival rate. Dates, ticks, text and expected rows are deterministic.
The scope excludes HTTP, roles, old pins, rolling deployment and schema evolution.
adtb query calls are not counted as write transactions. SQLite query calls use
BEGIN/COMMIT; writes use BEGIN IMMEDIATE. Include these actual API/encoding costs.

adtb uses serializable operation semantics, Local acknowledgement, internal
retry=0, explicit Pool with four I/O workers and four execution workers, a
64MiB page pool and process query reservation `writers * 64MiB`. Each write
success must report one internal attempt. Actors share database/runtime handles;
background scrub is disabled. SQLite uses one private connection per actor,
8192-byte pages, total 64MiB page-cache allowance split across actors, mmap=0,
foreign_keys=ON, ignore_check_constraints=OFF, read_uncommitted=OFF, WAL,
synchronous=FULL and wal_autocheckpoint=1000. Setup connection closes before
release. Native busy_timeout starts at 5000ms and is capped before each call by
remaining actor budget. Settings are asserted at connection startup; later caps
use the safe API without independent readback. Native busy-handler retry counts
are unknown. SQLite also has one idle Pool worker for the common clock adapter.

Outer retries apply only to a specific rollback-confirmed call, at most 100
attempts with 1ms backoff. Never repeat an acknowledged lifecycle prefix or an
uncertain write. The actor queue has a cooperative 120-second budget after
preparation/release; adtb statement limit is min(10 seconds, remaining budget).
SQLite's remaining-budget limit bounds busy waiting, not statement/commit I/O.
These mechanisms differ in scope. Do not alter durability, constraints or retry
policy to make a slow cell finish.

## Preparation, cache and identity

Every invocation starts a fresh process, creates its own database, loads the
fixture, validates the complete loaded state and prepares actor handles before
release. No prepared base images are cloned. There are zero additional warmup
jobs and no extra population scan beyond actual validation. Load and validation
naturally prime caches. Shared OS cache state is uncontrolled across invocations;
paired ordering balances order but does not equalize residency. Neither cold
cache nor stable warmed steady-state is claimed. SQLite uses OS page cache;
adtb uses direct I/O. Equal declared page-cache budgets do not imply equal RSS,
query memory or total resource consumption.

Use one reviewed source tree and one release executable for all peers/cells.
Before the exclusive run window, capture an immutable local build receipt with
full jj source identity and dirty diff/status, tracked-source hash manifest,
Cargo.lock/toolchain/manifests, build command/profile/features, Rust and C
compiler identities and flags, relevant Cargo/cc environment and executable
SHA-256. Hash the three schema/bundle files and both execution/report scripts.
Do not confuse a launch-tree revision with proof of an arbitrary binary's source.
Retain exact generated-source/build inputs when available. Build once:

```sh
cargo build --release -p adtb-bench --bin adtb-bench --features native --locked
sha256sum target/release/adtb-bench Cargo.lock rust-toolchain.toml \
  crates/adtb-bench/fixtures/log/0001.adt \
  crates/adtb-bench/fixtures/baseline-1.0.adb \
  crates/adtb-bench/fixtures/schema.sql work/tools/bench-run work/tools/bench-report
```

At build time unset LIBSQLITE3_SYS_USE_PKG_CONFIG, SQLITE3_LIB_DIR,
SQLITE3_INCLUDE_DIR, SQLITE3_STATIC, LIBSQLITE3_FLAGS and every SQLITE_MAX_*
override. Record compiler inputs explicitly. The locked native graph uses
rusqlite0.41.1/bundled libsqlite3-sys0.39.1. Every SQLite connection checks linked
version3.54.1 and source ID
`2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76`.
Retain runtime compile options; the installed sqlite3 CLI is not peer identity.
Both peers must retain exactly the expected actual settings and generator
`baseline-v1`. Any binary/source/config change starts a new protocol/sample set.

## Freeze and finite execution

The adopted common sizes are N=1000/5000, writers=1/4 and both workloads.
Counts remain 20,000 counter calls
or N lifecycle workflows. The two sizes span five times the primary cardinality.
At N=5000 the counter has four full key passes, rotating each key through all
four actors; N=1000 has twenty passes. No per-peer adaptation is permitted.

The reviewed runtime size gate requires N >= 4, N divisible by 4, N dividing
20,000, and 20,000/N >= 2, with writers1/4. These conditions enforce the chosen
coverage contract: equal campaign queues, integral closed-form per-key visits
and at least two ownership-rotating passes. They imply N <= 10,000 and
coprimality with stride37. Lifecycle shares this common size domain by protocol
choice. `counter-smoke` additionally requires N=4; smoke is not a meaningful
replacement for a failed larger cell. This gate replaces the old size enumeration
without changing queues, adapters, generator, oracle, counts or settings.

The original N1000/10000 calibration used30-second active runtime,1610612736
charged memory bytes, zero swap and64tasks. Retain the four30-second failures
(both N1000 serial counters and both N10000 serial lifecycles), the later
N10000 serial adtb120-second timeout and every other diagnostic. Never relabel
these outcomes as successes at another budget/size. Six successful N1000 cells
(four lifecycle cells and two serial counters) used the original executable
SHA-256 `4ca6f593baf407cfc4ce0595748f8f3dea6357e1c18ee990ec150cdf4db6f4a4`.
They remain explicitly old-binary feasibility evidence; reviewed parity establishes
that their accepted gate path and downstream production/config/schema/bundle
inputs are unchanged. It does not make them final-binary ranked samples.

The frozen code revision is
`2f0369f6875073286513ae150e94d113a38f25b7`. Its ten reviewed gate-source files
and modes match tested candidate `c1ec197c35766b1ce578c63b2f9b03f16c47f5c2`;
the landed revision additionally inherits two ticket Markdown files. The new
release executable SHA-256 is
`d64d20c6f4411b33a460262367d0b8ee8b3cdaa779cd9a570b43d3515b861e67`.
Ten selected cells use that executable, alongside the six explicitly old-binary
N1000 cells above. Every selected cell completed native/report validation. In
total the 16 cells contain 256,000 completed calls, 256,045 attempts, 208,000 writes
and 24,000 completed workflows. Maximum observed whole-command caller time was
60.492448356 seconds; conservatively subtracting it from the 120-second service
cap leaves 59.507551644 seconds. Maximum charged peak was 71,962,624 bytes. This
finite observed headroom supports the frozen bounds without guaranteeing future
completion. All calibration stays outside the ranked sample set, with source/BIN
identity per invocation. The predeclared N=2000 discriminator is unnecessary after that first N=5000
success and is not part of the frozen matrix. The future baseline
uses one final verified executable within the session for every cell, including
all N1000 repeats. Per-session executable identity is pinned alongside code and
build configuration; reproducible attempts on other hosts/build paths record
their own actual executable hash rather than requiring byte-identical binaries.
If only protocol Markdown changes after the verified gate build, bind that
unchanged code/build identity and actual executable hash to the final protocol
source revision rather than claiming the executable was rebuilt from it.

The common bounds are 120 seconds of native active runtime, 1610612736 charged
memory bytes, zero swap and 64 tasks for every sample. These bounds were adopted after
all 16 selected cells passed; no failed/nonselected cell is silently discarded.
For80invocations the maximum configured active exposure is9600seconds, plus up to176
seconds for configured2-second startup and0.2-second stop allowances:9776seconds
(2h42m56s). Reserve an exclusive180-minute window with a latest-launch cutoff at175minutes.
The final5minutes allow the last service budget (at most122.2configured seconds)
and closeout under the stated native liveness preconditions. The17m4s beyond
configured matrix exposure accommodates reporting/context/operator work. If
scheduling headroom is insufficient, retain remaining samples as not started
instead of extending the window or budgets. One prior build may reserve300seconds
separately. This operational scheduling limit is not an unconditional total
wall-clock guarantee: native manager/kernel/filesystem calls can stall, and
control-plane/reporting work is outside the service. A command already running
at launch cutoff remains under its native service budget. No supervisor is added.

The future measurement owner runs alone: no concurrent builds/tests/other
benchmark actors. Record uptime/load and process inventory before/after each
invocation, retain unexpected-load evidence and invalidate affected runs. Check
source/BIN hashes against the frozen receipt before/after each paired round.
Record an external session manifest binding protocol/source/BIN/config hashes,
run ID, repeat, cell/peer order, native invocation ID, directory, declared
preparation/cache policy, budget and status. This manifest is authoritative for
membership in the frozen session and repetition/preparation policy. The producer
does not encode repeat numbers. Raw report `context.measurement_policy` retains
wrapper-default cache/warmup/repetition/classification, including its reporting-run
classification. Those fields describe wrapper knowledge, not absence of the
externally frozen session. Keep them untouched and preserve the manifest alongside
reports; do not invent an observed cache state or change reporter code.

Use Bash in the development environment. Set SESSION to a new local artifact
directory and SECONDS_LIMIT/MEMORY_BYTES/TASKS to120/1610612736/64,
and keep this exact order. This shell recipe is orchestration of existing tools,
not a new executable driver. All-success runs continue automatically. Any nonzero
native/report outcome pauses further launches for owner classification; remaining
rows stay visible as not-started-review.

```bash
: "${SESSION:?set SESSION to a new nonempty artifact directory}"
if ! mkdir -- "$SESSION"; then exit 2; fi
launch_cutoff=$((SECONDS + 175*60))
pause_launches=0
printf 'run_id\trepeat\tcell\tpeer\tstatus\trun_exit\treport_exit\n' > "$SESSION/index.tsv"
cells=(counter:1000:1 counter:1000:4 counter:5000:1 counter:5000:4
       lifecycle:1000:1 lifecycle:1000:4 lifecycle:5000:1 lifecycle:5000:4)
for repeat in 1 2 3 4 5; do
  for position in 0 1 2 3 4 5 6 7; do
    cell_index=$position
    if ((repeat % 2 == 0)); then cell_index=$((7-position)); fi
    IFS=: read -r workload n writers <<< "${cells[$cell_index]}"
    peers=(adtb sqlite)
    if (((repeat + cell_index) % 2 == 0)); then peers=(sqlite adtb); fi
    for peer in "${peers[@]}"; do
      run_id="r${repeat}-c${cell_index}-${peer}"
      run="$SESSION/$run_id"
      if ((pause_launches)); then
        printf '%s\t%s\t%s\t%s\tnot-started-review\tNA\tNA\n' \
          "$run_id" "$repeat" "${cells[$cell_index]}" "$peer" >> "$SESSION/index.tsv"
        continue
      fi
      if ((SECONDS >= launch_cutoff)); then
        printf '%s\t%s\t%s\t%s\tnot-started-cutoff\tNA\tNA\n' \
          "$run_id" "$repeat" "${cells[$cell_index]}" "$peer" >> "$SESSION/index.tsv"
        continue
      fi
      if work/tools/bench-run "$SECONDS_LIMIT" "$MEMORY_BYTES" "$TASKS" \
          target/release/adtb-bench "$peer" "$workload" "$run" "$n" "$writers"; then
        run_exit=0
      else run_exit=$?; fi
      if work/tools/bench-report "$run"; then report_exit=0
      else report_exit=$?; fi
      if ((run_exit != 0 || report_exit != 0)); then pause_launches=1; fi
      printf '%s\t%s\t%s\t%s\tinvoked\t%s\t%s\n' "$run_id" "$repeat" \
        "${cells[$cell_index]}" "$peer" "$run_exit" "$report_exit" >> "$SESSION/index.tsv"
    done
  done
 done
```

Before execution, bind this frozen cell list and protocol hash in the session
manifest. Any change requires a new explicit freeze decision. The owner must supply the
load/process checks and source checks around this recipe; these
are scheduling/evidence duties, not implemented promises of bench-run. Never
reuse a run directory. Failed native commands still need bench-report. The
exclusive successful directory creation establishes ownership before the index
is written; an unset/empty SESSION or existing/inaccessible directory aborts. The
index records every planned invocation, including those never launched at cutoff;
not-started rows have no native invocation, report or fictitious performance value.
After classifying a resource/external failure, the owner may explicitly resume the
unchanged remaining plan within an approved exclusive window. Preserve the first
index and link the resumed plan to it; launch only never-invoked IDs. Resumption
never retries the failed invocation implicitly. Correctness failures stop the set.

## Acceptance, failures and reporting

A complete sample requires native normal exit0, matching invocation/limit/BIN
receipts, successful report exit0, complete planned calls/workflows, decoded
expected outputs, compatible counter history and all loaded/before-close/
after-reopen/closed-form gates. Check counts against the table above independently
of computed rates. Local acknowledged-write durability must remain established
by reopen and the retained independent ACK-journal controls. These finite controls
do not simulate power loss or prove arbitrary hardware honors flushes.

Timeout, OOM, task refusal, uncertainty, incomplete work or missing/corrupt evidence
is retained invalid/incomplete with no comparable rate. A killed command can lose
its buffered ACK ledger: its prefix is unavailable and cannot be called validated
censoring. Keep database, stdout/stderr, ledgers, service receipts and failure
reason. An acknowledged write absent after reopen or impossible outcome is a
correctness failure: stop collection and tuning, investigate, then invalidate the
affected set. Do not count errors or partial lifecycles as useful completions.

No automatic replacements. A verified external interruption/load violation or
collector fault may receive one explicitly approved replacement under unchanged
inputs/budgets, in a new directory linked to its original run and reason. No
replacement for genuine timeout, OOM, retry exhaustion or poor throughput. Keep
all originals visible. Cells without five complete planned repetitions publish
their outcomes and available diagnostic samples; do not present a five-run
comparative median or silently drop failed peers/cells.

Preserve all raw calls/attempts/workflows and complete report.json/report.txt for
every invocation. Publish each run's rates/counts, p50/p95/p99 for completed,
failed and uncertain calls separately and completed workflows, retry/refusal
rates with their stated denominators, and all five observations plus median and
min/max for each complete cell. Do not pool raw samples across runs as if they
were one run or select the best run. No across-workload headline aggregate.

Latency includes waits, retries and backoff from each actual offer through decoded
terminal outcome. The steady window is release through last terminal call.
Lifecycle throughput, write-call throughput and all-call throughput have distinct
units. Nearest-rank percentiles use sorted integer nanoseconds, rank ceil(p*N).
Report setup, load, preparation, validation, close/reopen and artifact phases
separately; overlapping actor phases cannot be added.

Native CPU/memory/I/O cover the whole service through capture, including setup,
validation, wrapper and stop helper. Memory peak is charged cgroup bytes including
page cache/kernel charges, not peak RSS or steady-phase memory. CPU is microseconds.
I/O is per-device accounting; do not sum stacked devices as independent physical
writes. Unavailable counters retain their reason, never become zero. The narrower
/proc/self/io fallback covers only CLI process/threadgroup through final close
before artifact writes. Neither source promises physical-media traffic. Active
storage inventory is after actor drop and may follow SQLite checkpointing; it is
not peak WAL. Closed inventory follows reopen validation/final close. Exclude
report/ledger files from database footprint. See [reporting.md](reporting.md).

Retain machine paths, hardware/process inventory and raw host context locally.
Publish portable protocol/configuration, approved environment description,
source/BIN hashes, workload-specific results, failures and unknowns. Review raw
artifacts for private host details before public attachment.

## Superseded proposal fields

Earlier preparatory reports are evidence of proposals, not additional requirements.
This protocol retires prepared-base cloning,100-job disposable warmup, extra
population scan, all-at-release scalar offers, every-fourth counter ownership,
fixed64MiB query reservation at four writers, SQLite busy_timeout0, steady-resource
snapshots, peak RSS/peak WAL and a120-second hard timed-work watchdog. The current
crash controls use N4/eight calls and four exact cuts, not100 acknowledgements.
Build/report/startup/load costs are retained with actual scope. Any unsupported
old field stays explicitly unmeasured rather than being invented in results.
