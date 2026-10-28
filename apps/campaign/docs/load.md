# Deterministic campaign collisions

Run the workload from the repository root inside the dev shell:

```sh
cargo nextest run --manifest-path apps/campaign/Cargo.toml \
  --config-file .config/nextest.toml --test load
```

The default and quick profiles run seed 4. The deep profile runs seeds 4
through 11 through the shared `adtb_testkit::profile::sweep` mechanism:

```sh
cargo nextest run --manifest-path apps/campaign/Cargo.toml \
  --config-file .config/nextest.toml --profile deep --test load
```

Each seed starts from the same simulated-file population: two campaigns,
one party and four NPCs per campaign, two known targets per party, and an
underway encounter with two extras in the first campaign. Migration replay
and registration use the complete unchanged log and `campaign-1.6` bundle.
The population is created once and its file bytes are copied before each run.

Each collision starts two GM actors and a player reader through the engine's
simulated runtime. Page-memory switching is enabled at probability 0.3.
The actors make eight turn advances, two session plans, two competing session
starts, and two reveals and two entity edits in each campaign. The player
reads entity lists, shared detail and encounters during each phase. Only the
engine retries writes; the workload records its reported attempt counts and
requires an observed retry in every tested seed. A start gate supplies
concurrency opportunities; it does not prove a retry for every operation.

The operation ledger records actor, action, typed target key, per-actor step,
commit position, attempts and returned value or exact refusal. The checks
compare encounter turn/round to acknowledged turn calls, session numbers and
the live session to committed plans/starts, and each entity's text, summary,
secret and both link sets to the last edit by commit position. Duplicate
reveals must leave one discovery. Player entity keys must belong to the
party's known set. Phase expectations require each source to be absent before
its reveal, optional during that reveal, and present afterwards. Unknown and
other-campaign details must be empty. Each party sees exactly its fixture
encounter and roster, or no encounter for the other party, and
secret sentinels must be absent. Hidden link targets stay out of player links
although those targets are known. All final checks run again after close and
reopen in a new simulated process.

Every seed runs twice from the same initial bytes. Its operation ledger,
scheduler trace and resulting database bytes must replay exactly. Nextest
runs retain both passes' `.ledger` and `.trace` files below
`target/tmp/campaign-load/<NEXTEST_RUN_ID>/`; Cargo's target directory controls
the prefix. Plain `cargo test` uses the `cargo-test` subdirectory. Failures
include the seed and evidence path. Preserve a failing run before another
plain Cargo invocation, which can replace those files.

A local verification run on 2026-10-28 took 11.050 seconds for the default
seed with both passes and 64.946 seconds for the eight-seed deep sweep. These
are test execution times, excluding compilation. The default is one small
sweep and exceeds the usual two-second target for an individual test.

This is a bounded correctness workload. The full design population of 200
campaigns, large texts, many sessions/quests/events, Zipf distributions,
kill-versus-roster collisions, continuous randomized actors, injected storage
faults/crashes, and latency targets remain unimplemented. It does not replace
all constraint checks or a full isolation-history checker. Simulated time is
not a measurement of native request latency; no p99 result is claimed.
Suspected engine failures stop application expansion and require a reduced
engine regression and general fix.
