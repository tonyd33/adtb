# Native SQLite comparison: correctness checkpoint

This opt-in consumer exercises the public generated adtb bundle API and
rusqlite prepared statements on real files. It contains no measurements,
pilot, benchmark ranking, or optimization claim. The complete comparison
protocol and measurement reservation remain separate work.

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

Outstanding before measurement readiness: externally flushed acknowledged-prefix
child-kill controls using these adapters and the remaining comparison protocol
review. The buffered ledger is insufficient to certify a killed process. Resource
counters and latency distributions are unavailable; there is no ranked
result. Large dataset feasibility is unverified.

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
its database files. These are correctness operations, with no warmup or
timing report. N=4 is a smoke fixture; N=1000/10000 are planned datasets.

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
no hard process deadline. A parent watchdog, statement interruption and
bounded drain/kill containment remain required before measurement readiness.
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
