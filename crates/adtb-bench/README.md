# Native SQLite comparison: correctness and reporting

This opt-in consumer exercises the public generated adtb bundle API and
rusqlite prepared statements on real files. It retains operation samples and
correctness evidence for native reporting. Ranked dataset measurements,
benchmark ranking and optimization remain separate work.
The [frozen executable comparison protocol](protocol.md) defines the paired
N=1000/5000, one/four-writer matrix and actual preparation/cache semantics.
All 16 feasibility cells passed; no ranked baseline has been collected.

The keyed control uses N counters initially zero and 20,000 atomic
read/increment/return requests. The reduced campaign workload has four
campaigns, four Party entities, four Npc entities and N Planned sessions.
Each session runs start, joined Live read, finish, keyed constructor-match
Played read. IDs follow `1 + ((i * 37) mod N)`; four lifecycle actors own
one campaign each. Counter request i belongs to actor
`(i + floor(i/N)) mod writers`; each pass rotates key ownership across actors.
This permits shared-key access without guaranteeing simultaneous collisions.
The original every-fourth assignment failed to share keys for these sizes
and was corrected before any measurement freeze. Dates, ticks, text and keys are deterministic. This
covers a campaign-derived subset, with owner authority and one current
schema pin. It does not exercise HTTP, roles or schema evolution.

SQLite uses STRICT tagged rows, branch CHECKs, a composite Party/campaign
foreign key, a partial unique Live index and an irreversible Played
transition trigger. These stay active during calls. Each actor retains
its prepared statements across calls. Successful calls consume and decode
all rows and acknowledge only after COMMIT. Retries apply only to the
individual call after a known rollback, never to an acknowledged lifecycle.
COMMIT uncertainty invalidates the operation and permits only the identified
in-flight whole write in the independent reopen oracle.

The current checkpoint validates exact generated outputs, basic transaction
precondition refusals, the complete state including support relations,
and reopened state. The ledger distinguishes actor/call IDs, attempts,
acknowledgements, returned values, errors and adtb commit positions.
Incomplete runs fail the command after validating acknowledged prefixes;
partial work is never reported as successful complete throughput.
Counter validation preserves actor call order and returned increment-value
order across all keys and requires a compatible serialization. Known values
must be unique and the full final state must account for every acknowledged
increment. Missing values may belong only to identified terminal uncertain
calls. At most four actors with one uncertain call each bound the assignment
search to at most 4! fillings; unassigned uncertain calls did not commit.
Each filling must admit an acyclic union of actor and per-key value order.
This also rejects cycles that span different keys. An independent finite
serial-interleaving oracle checks the history boundary.

The runtime-loaded schema matrix in `fixtures/controls/` checks declarations,
keys, references and Party subsets, campaign membership, scoped Live uniqueness,
constructor shape, Played transitions, least-name selection and whole-change-set
atomicity through the existing real-file adapters. Thirty-three shared cases and ten
SQLite-only tagged-shape cases assert exact
outcomes and complete typed snapshots before, after and after close/reopen.
Rejected SQLite writes also preserve every stored column of all four tables,
including encoding columns unused by the current constructor.
Successful native unit writes return no rows; SQLite reports the expected
last-statement affected count. Case names, section names, family counts and
reached floors are checked. Positive controls cover valid keys/references,
matching campaign/Party updates, forward transitions, Played edits, different
campaign Live rows and Planned rows beside a Live row.

Generated ADTB enums exclude malformed tags and branch fields at Rust type
checking. Raw native shape controls instead assert exact ad hoc admission
refusals. Neither is counted as a native runtime constraint refusal. Typed
native parameters can carry a nonexistent or non-Party key; these inputs reach
runtime. The membership check dereferences Party, so dangling, non-Party and
inbound retag/delete controls fault with `NoSuchRow` before reporting a foreign
key violation. SQLite returns its native extended constraint code and message;
it provides no ADTB-style violation report or canonical-name selection.
With the same competing counter/campaign key writes, native selects the least
canonical constraint name; SQLite stops at the first failing SQL statement.

An ADTB retag or same-key delete/insert in one change set corresponds to one
SQLite UPDATE. Separate SQLite DELETE and INSERT statements have separate
constraint boundaries, as do separate ADTB statements; the transition trigger
makes no transaction-wide promise. The direct sessions.campaign reference is
logically implied by a valid Party reference, the Party's campaign reference
and the named equality check. SQLite's composite Party/campaign reference plus
entity.campaign reference imply its direct session.campaign reference too.
Removing either redundant direct declaration alone therefore need not change
accepted states. Sensitivity witnesses distinguish individual obligations from
compound removal of redundant guards.

Native and SQLite unscoped-unique controls run normally with exact names,
reports and state assertions. Native constraint names come from the shared
schema naming boundary.

The opt-in native tests exercise an externally retained acknowledged prefix
through child termination and reopen on both adapters. They are a bounded
correctness checkpoint described below. The bounded command entry provides
operational containment. The [frozen protocol](protocol.md) records the selected
feasible matrix and retained timeouts. The buffered command-line ledger cannot certify a killed
process. The reporting entry below combines latency samples and native resource counters
with command validity. There is no ranked result.

## Build and checks

Run from the repository root in the development environment. The default
feature set is empty. Root default build, lint and test commands skip the
native binary, code generator and bundled SQLite C compilation. They only
compile the small empty library and build-script entry point. Dependency
policy inspects all features, including this opt-in graph.

```sh
cargo check -p adtb-bench --locked
cargo clippy -p adtb-bench --all-targets --locked -- -D warnings
cargo clippy -p adtb-bench --all-targets --features native --locked -- -D warnings
work/tools/suite -p adtb-bench --features native
cargo deny check
cargo fmt --all --check
work/tools/house-check
work/tools/size-check.sh
```

The native smoke test reads its fixture at runtime. It checks exact outputs
and refusal values against expectations constructed independently of peer
results, then checks every stored relation before and after reopen.

The native dependency is rusqlite 0.41.1 with default features disabled and
only `bundled` enabled. Cargo.lock pins libsqlite3-sys 0.39.1. No cache,
custom FFI, build-time bindgen, system-library or SQLCipher feature is used.
At build time unset `LIBSQLITE3_SYS_USE_PKG_CONFIG`, `SQLITE3_LIB_DIR`,
`SQLITE3_INCLUDE_DIR`, `SQLITE3_STATIC`, `LIBSQLITE3_FLAGS` and every
`SQLITE_MAX_*` override, and record compiler/CFLAGS inputs in a local
receipt. Build with the committed lock. Runtime startup checks version
3.54.1 and its exact source ID and records all compile options.

## Explicit correctness operations

Each command requires a new directory and retains `ledger.txt` alongside
its database files. These are correctness operations with raw timing artifacts
and no warmup; the bounded recipe below joins native resources into a report. N=4 is a smoke fixture. Supported runtime sizes have four equal campaign partitions
and at least two whole passes of the fixed 20,000 counter requests: N must be at
least 4, divisible by 4, divide 20,000, and satisfy 20,000/N >= 2. The same size
policy applies to lifecycle; writers must be 1 or 4. These conditions imply
N <= 10,000 and make stride 37 a key permutation.

```sh
cargo run -p adtb-bench --features native --locked -- adtb lifecycle target/adtb-lifecycle 4 4
cargo run -p adtb-bench --features native --locked -- sqlite lifecycle target/sqlite-lifecycle 4 4
```

The `counter-smoke` operation uses eight fixed requests with N=4. For example:

```sh
cargo run -p adtb-bench --features native --locked -- sqlite counter-smoke target/sqlite-counter-smoke 4 4
```

The ordinary `counter` operation retains the fixed 20,000-request protocol.
It can stop incomplete when an actor observes its 120-second queue budget.
Do not use partial work for comparisons or change limits until a separately
approved protocol freeze.

One actor owns each connection and keeps at most one call in flight.
Attempts are bounded at 100 per call with a 1ms outer backoff. Each actor
starts a cooperative 120-second queue budget after preparation and the
start barrier, checking it before each attempt. adtb receives a statement
time limit capped by the remaining actor budget. SQLite's remaining-budget
cap applies to its lock wait, not statement execution or commit I/O. Setup,
preparation, blocked calls, joins, snapshot/reopen and ledger writes have
no hard process deadline when invoked directly. Use `bench-run` for the native
whole-command boundary described below.
The earlier 20,000-request diagnostic stopped cooperatively, not under a
hard 120-second parent ceiling. Fatal or ambiguous outcomes request
cancellation of further queued work; existing calls return before the
ledger is retained and the database reopens for the oracle.

adtb explicitly selects Pool with four I/O workers, Local acknowledgement,
retry=0, a 64MiB page pool and process query reservation of
`writers * 64MiB`. The latter follows the engine's fixed 64MiB per-call
grant and one in-flight call per actor: 64MiB for one actor, 256MiB for
four. Open-time settings and disabled background scrub are recorded.
A retained initial four-actor run with a 64MiB process reservation refused
three concurrent starts with a memory admission limit; its acknowledged
prefix survived reopen.

SQLite uses private connections, 8192-byte pages, a total 64MiB page cache
split across actors (the setup connection closes before release), mmap=0, foreign_keys=ON, ignore_check_constraints=OFF,
read_uncommitted=OFF, WAL, synchronous=FULL and wal_autocheckpoint=1000.
Each connection uses native busy_timeout=5000ms, capped to the remaining
actor queue budget before each call. This wait belongs to the call; native
busy-handler retry counts are unknown, while outer BEGIN attempts are
recorded. The earlier zero-wait concurrent control exhausted its 100 outer
attempts and remains an incomplete diagnostic, not an unsupported workload.
Runtime startup reads back these settings on every connection. Later lock-wait
caps use the safe busy_timeout API and are not independently read back.
These
mechanisms have different scopes: neither cache budget nor adtb query
reservation establishes equal process RSS. SQLite uses the OS page cache;
adtb uses direct I/O. Close/checkpoint costs are not measured here. A common bounded host-clock
adapter uses the public runtime clock interface; SQLite correctness runs
currently have one idle Pool worker for that clock. Its process overhead
is included if whole-process resource counters are later collected.

## Acknowledged-prefix child termination

The two native `acknowledged_prefix` tests read `fixtures/crash.txt` at runtime.
For each adapter they execute the fixed N4/eight-increment smoke sequence with
four cuts: after ACK3, after ACK8, before the next call after ACK3, and after
that next call commits with its response withheld. These last two cuts exercise
the absent and present sides of one identified whole pending write. Withholding
a response is harness transport uncertainty, not native flush-failure injection
or a native ambiguous-commit error.

A writer child uses the existing real-file adapter. An ACK is a committed
Number result received by the parent and synced to the parent's own journal
before it permits more progress. The exact cut handshake must arrive before
the parent kills and reaps its owned writer PID. A separate verifier child
reopens the file and checks every relation against external expected values,
including the empty support relations. It also runs the retained independent
counter-history oracle against the journal, with at most the identified terminal
call uncertain. Both a validation receipt and successful verifier exit are
required. The parent performs no database calls.

Each case gives child work and socket/exit coordination one absolute ten-second
deadline; partial socket transfers recompute the remaining time. Error cleanup
kills the owned PID and polls for reap for at most two additional seconds.
Children run sequentially through the heap-cap helper under the parent's exact
slots-2 nextest assignment, with one slot each. Child output goes to owned
files, so no pipe-drain thread can outlive the parent. Journals, child logs and
database files remain under `target/sqlite-crash-controls/cases`.

This bound assumes the parent's filesystem and process syscalls return. Journal
flush, file creation and process spawn have no interruptible standard-library
deadline. The tests provide no universal wall-clock watchdog and do not add
hard containment to the command-line operations. They establish this finite
crash-control checkpoint, not measurement readiness or engine-wide crash
coverage.

## Bound command execution

`work/tools/bench-run` runs an already-built native CLI in an owned transient
systemd user service. It never builds or retries a command. Supply positive
runtime seconds, memory bytes and task budgets, then the existing CLI arguments:

```sh
cargo build -p adtb-bench --bin adtb-bench --features native --locked
work/tools/bench-run 5 1610612736 64 target/debug/adtb-bench sqlite counter-smoke target/sqlite-bounded-1 4 1
```

The memory and task values are caller policy, with no implicit default. The
cgroup memory budget includes resident C allocations and page memory, unlike
the test heap allocator; the two limits do not establish equivalent resources.
The example is a tiny correctness operation, not a measurement protocol.

The run directory must be new. Its `database/` is passed to the unchanged CLI;
`stdout`, `stderr`, native manager/journal logs, invocation and actual cgroup
limit receipts, exit status and `result` remain alongside it. Startup refuses
actual limits different from the requested memory/task budgets or zero swap.
The manager bounds active runtime by the supplied seconds, executable startup
by two seconds, and receipt completion by two seconds. At the active deadline
it sends SIGKILL to the entire service control group. The protocol counts all
five potential two-second native stop states separately. It owns all
service descendants through their cgroup, including a new process session.
No pipe-drain thread or Rust supervisor is added.

Only native service success with normal exit0, matching invocation receipts and
successful manager wait marks `result` complete. The CLI owns its existing exact
state/history/reopen checks. Timeout, OOM, signal, refusal or missing evidence
leaves the run invalid/incomplete. A killed CLI's buffered ACK ledger is not
crash-safe: its acknowledged prefix is unavailable and is not validated here.
Retain partial state for diagnosis; never retry over that run directory.

The tested host is Linux with systemd262, unified cgroupv2, delegated memory/pid
controllers and a reachable user service manager. Bash, jq and coreutils are
required. Missing native facilities fail rather than launch without containment.
The bound assumes manager/kernel/filesystem/process syscalls return; it is not
a universal wall-clock guarantee or measurement-readiness certification.

Run the opt-in facility integration tests explicitly after building the CLI:

```sh
crates/adtb-bench/fixtures/run-boundary-test target/debug/adtb-bench
crates/adtb-bench/fixtures/run-boundary-test --fixtures-only
crates/adtb-bench/fixtures/receipt-deadline-test
```

These tests additionally require GNU timeout, setsid and Python3. They exercise
wrapper wiring, exact exits, resource refusal, descendant termination, retained
incomplete results and tiny real-peer commands. They are deliberately outside
the systemd-free global test commands. The prior finite crash-control tests
remain the independent externally retained ACK validation checkpoint.
The fixtures-only mode omits peer commands. The receipt deadline fixture uses
owned scratch runner copies to delay the real receipt by 500 ms and proves
that the old 200 ms allowance loses it. A TERM-ignoring parent and session-detached
descendant demonstrate that raising only the stop timeout permits late effects;
the repaired hard native kill prevents both effects.


## Report a bounded run

Build once, then run an explicit report command after each bounded command.
Python 3 from the standard library is required for reporting. These tiny runs
check reporting and correctness; they are not a comparative baseline. Choose a
new directory for every run, including failures.

```sh
cargo build -p adtb-bench --bin adtb-bench --features native --locked
work/tools/bench-run 10 1610612736 64 target/debug/adtb-bench sqlite counter-smoke target/report-sqlite-counter 4 1
work/tools/bench-report target/report-sqlite-counter
work/tools/bench-run 10 1610612736 64 target/debug/adtb-bench adtb lifecycle target/report-adtb-lifecycle 4 1
work/tools/bench-report target/report-adtb-lifecycle
```

`bench-report RUN` writes versioned `report.json` and readable `report.txt`, and
prints the readable report. Run it on failed directories too: incomplete, timed
out, OOM, corrupt or missing evidence returns nonzero and suppresses comparable
throughput. Native success, invocation identity, complete raw operation coverage
and every application validation must agree before a report is complete.
Diagnostic counts from valid partial application evidence remain visible; missing
or corrupt application evidence makes counts unknown. Do not retry over a run.

The [version 1 artifact contract](reporting.md) defines the raw application files
under `database/`: `application.json`, `calls.ndjson`, `attempts.ndjson` and
`workflows.ndjson`. The reporter recomputes counts and percentiles from those raw
records. Call latency starts before the first attempt and ends at the final
decoded outcome, including lock waits, retry attempts and backoff. Actors offer
one call at a time after preparation; future calls are not offered at release.
The steady window runs from common release to the last terminal call. Failed and
uncertain samples have separate distributions. Percentiles use sorted integer
nanoseconds and nearest rank `ceil(p*N)`; empty populations produce null.

Calls, successful write calls and completed lifecycle workflows have separate
counts and rates. N=4 lifecycle has 16 calls, eight write calls and four
workflows; a workflow completes only after its final Played read. A final
retryable failure does not create another attempt. Native SQLite busy-handler
counts and unobserved internal abort counts remain explicitly unknown. Refusal
and outer retry rates state their attempt or started-call denominator.

Phase spans retain open/configuration, load, preparation, steady operations,
validation, close/reopen and artifact costs separately. Preparation overlaps
across actors; do not sum overlapping phases. Storage inventories contain file
logical and allocated bytes at active validation and final closed boundaries.
The active inventory occurs after actor handles have dropped, so SQLite may
already have checkpointed; it is not a peak WAL inventory. Reporting files and
ledgers do not contribute to database footprint.

`resources/` retains raw `cpu.stat`, `memory.peak`, `memory.events` and per-device
`io.stat`, or a counter-specific `.error` reason, before the service unloads.
Invocation, cgroup, capture boundary and scope accompany these files. CPU values
are microseconds. Memory peak is charged cgroup bytes including page cache and
kernel charges. These counters cover the whole service through capture,
including setup, validation and wrapper/stop-helper overhead; they are not
steady-phase CPU or peak RSS. I/O records use bytes and operation counts per
block device; stacked devices must not be added as independent physical writes.
A readable empty `io.stat` means zero observed device I/O; an unavailable file is
not zero. When cgroup I/O is unavailable, the report retains its reason and the
application's `/proc/self/io` process/threadgroup fallback before artifact writes:
`read_bytes`, `write_bytes` and `cancelled_write_bytes` are storage accounting;
`rchar`/`wchar` count logical syscall bytes. Neither source promises physical
media traffic. The fallback excludes other processes and stop-helper work.

The wrapper retains the actual executable SHA-256, command, kernel, uptime,
filesystem and available launch-tree context. Application build fields originate
from Cargo; a launch-tree revision alone does not prove an arbitrary executable's
source. Machine context stays in local run artifacts. Cache state, warmup and
repetitions are not controlled by these tiny commands: retain that limitation
when interpreting their output, and freeze those inputs before comparisons.

Run the independent arithmetic, malformed-evidence and native collector checks:

```sh
python3 crates/adtb-bench/fixtures/reporting/test
python3 crates/adtb-bench/fixtures/reporting/collector-test
python3 crates/adtb-bench/fixtures/reporting/maximum-test
```

The collector check uses bounded synthetic services, requires the same native
facilities as `bench-run`, and never invokes a database peer. The arithmetic
fixtures are synthetic oracle values, not performance measurements.

The reporter limits each manifest, native counter or context input to 16 MiB,
and each streamed fixed-scalar NDJSON record to 4 KiB. This bounds individual
input decoding; it is not a process RSS limit. Record counts follow the unchanged
workload limits: up to 40,000 planned calls and 100 attempts per call. The maximum
input fixture checks healthy 20,000-call counters and N=10,000 lifecycle reports
with 40,000 calls and four actors. Oversized, nonregular or malformed evidence
produces an invalid diagnostic report with native outcome retained, rather than
reducing workload counts. Raw records are streamed; attempts are summarized per
call instead of retaining every retry in memory.
