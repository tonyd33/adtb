---
created-at: 2026-10-31
status: open
kind: bug
priority: 401
summary: acknowledged work and failure outcomes are not searched through Database under every fault
---

# Fault matrix

## Report

Sweeps sit at pager level. No `Database` workload runs under faults, and `Database::open` prefers io_uring, whose faults are absent. About 44 fixes and open bugs share two causes: a multi-step change not atomic under failure, and damage or failure giving a panic, hang or flattened error.

Build on the failing-and-crashing harness of backup round 4. For every workload (statement, commit, vacuum, reindex, scrub, verify, open, migration): fail each op, crash at each op, apply each rot kind to each page, on sim and on real backends through `Database`. Assert no panic and no hang, correct marks, acknowledged writes kept, reopen succeeds.

Then replace the fatal I/O panic classified by text (`panic_text`, storage/driver.rs, write/reindex.rs) with a typed payload, and give `Internal` and `Corrupt` a typed cause.

Close by reading the matrix: real-backend-faults-engine, heap-read-foreign-page-panics, space-append-spin, truncated-to-nothing, verify-sweeps-no-progress, catalog-open-error-nesting, deployment-refusal-sentinel, and parts of scrub-follow-ups and lost-bitmap-follow-ups.

From test-audit-a: no sweep reaches a commit refused after a quarantine mark, 0 refusals across 8 seeds under deep with 20 yields before commit. `a_commit_waits_for_a_mark_being_set_and_is_refused` covers one deterministic case. The matrix must reach this case, with a floor.

From test-audit-a round 2. `pager.verify` keeping only `Finding::Page` survives the pager floors, which count findings of all kinds: floor log and orphan findings by kind. `NoHeader` also stands for an I/O failure opening the data file, so `judge` cannot tell it from damage. The threaded findings floor is 1 of 3 measured, 0 under quick.

Candidate mechanism, from backup round 5: create a database by building file and log aside and publishing last, as restore does. A crash inside create then leaves no pair and a retry works, where today it answers `LogPresent` until files are deleted. Weigh the reopen it costs. Sweep every open with `create` through `Database::open`, the host path: tests through `open_in` never reached a failed open's removal, which deleted a restored log.

## 2026-10-31: first bounded slice, host-open provenance

Track `fault-open`, based on `38f07e0e`, starts at the actual host
`Database::open` path. The existing simulator operation search does not
intercept `api/file.rs`'s `std::fs` open protocol. Investigate
open-fallback-create-provenance before widening a sweep: an exclusive
create followed by a creating fallback may lose the fact that this call
created its inode.

Invariant: creation provenance comes from the operation that creates the
inode; opening an existing file cannot create one. Prefer the production
operation protocol as enforcer, with a deterministic search of its finite
race windows and exact refusal-cleanup assertions through public open.
First establish reachability and the smallest existing injection boundary;
do not introduce a general filesystem abstraction or process runner merely
to exercise this window.

Acceptance for this slice: reproduce the candidate or record why it cannot
occur; preserve unrelated existing files and acknowledged data; prove a
refused create cleans up what it made; show the regression fails with the
old protocol restored; run affected unit/public-open binaries and clippy,
independent review, then final and post-land `check`. Exact test filters
follow once the bounded test seam is established. The lead owns execution
and integration. This slice does not close the umbrella: other workloads,
real-backend fault injection, page rot and typed fatal causes remain open.

The read-only trace confirmed that the runtime is constructed after the
host lock/open protocol. The proposed fix uses a noncreating fallback and
returns its vanished-path result to the existing bounded acquisition loop.
A private test-only callback can expose the syscall window to tests that
call the actual public `Database::open`. A missing `slugify` implementation
required by a constraint supplies the exact post-log-claim refusal
`HostFnMissing { name: "slugify" }`. These are source-traced claims;
reproduction and mutation remain to run before claiming the defect fixed.

## 2026-10-31: host acquisition slice landed

`3177391253a874d60614f5ba87a2980cd12d8d13` fixes the creating-fallback
ownership race through a noncreating open and typed retry in the existing
bounded acquisition loop. The actual `Database::open` test checks four
finite pathname cases with exact window counts, refusal, retention,
inode and sentinel assertions. The old protocol and an opposite false
ownership mutation both failed those assertions; restored checks passed.
A strengthened real-file control separately preserves acknowledged
`ann` across an exact refused migration and reopen.

Independent reading review found no introduced blocker. Final and
post-land full `check` both passed: 3755 root, 48 campaign, 13 Loom and
39 Miri tests, plus all configured static, documentation, grammar and
tooling checks. The narrow ticket open-fallback-create-provenance is
resolved; the preexisting read-only acquisition cleanup candidate is
tracked in open-acquisition-create-cleanup.

This umbrella stays open. The private test seam reaches host syscalls
before a runtime exists; it does not replace runtime operation search or
prove the full fault matrix. Next select a bounded statement/commit
operation-failure workload using the existing search harness, checking
acknowledged rows and exact outcomes with reached-case floors. Other
workloads, crash/rot coverage, real backends and typed fatal causes remain.

## 2026-10-31: statement/commit search boundary

Track `fault-commit`, based on `6128e7f9`, is tracing a bounded workload
through `Database` and the existing `simkit/search.rs` operation search.
The oracle must preserve each recorded successful acknowledgement on
reopen and permit an unacknowledged transaction only as a whole where
the contract allows it. Reached-operation and outcome floors must make
an empty or unexercised search fail.

The existing `each_failure` requires a normally finished simulator run;
write/flush failure is instead a required engine stop. Its current
crash helper and maintenance catches identify fatal I/O by message
prefix. Investigate a typed fatal cause before extending the search, so
an ordinary assertion panic cannot count as an expected crash. Follow
every producer/consumer and the simulator's panic/cancellation path;
do not add a separate text heuristic at a new entry point.

This phase is source investigation, not execution evidence. Select one
modest search with its prerequisite, or land the minimal prerequisite
first if that keeps the ownership boundary clear. No broad crash/rot,
real-backend or known unbounded-directory test is authorized by this
slice alone. Final acceptance still requires measured regression or
mutation, affected checks, fresh review, and final/post-land `check`.

The source trace selected `fault-fatal` as a separate prerequisite:
migrate existing Driver/Log producers to one typed fatal-I/O payload
carrying actual `IoError` and diagnostic context, and migrate shared
maintenance/test-helper classification to type identity. Simulator core
already crashes storage and joins threads before forwarding the payload;
its lifecycle needs no new special case. An unexpected successful I/O
completion remains an invariant failure, not a fabricated I/O cause.

Acceptance for the prerequisite: a lookalike ordinary panic must not be
accepted as fatal; genuine injected I/O must be recognized; ordinary
worker panic controls must remain usable; all current producers and
consumers are inventoried. Preserve useful diagnostics at the one raising
point without a global hook framework. No new process-search API or
acknowledgement workload is included in this prerequisite. Those follow
only after its typed boundary is verified and landed.

## 2026-10-31: fatal identity reproduced and catch boundary completed

The old helper accepted an ordinary owned string panic as
`Outcome::Crashed` solely because of its diagnostic prefix. The
regression on test-only `ed6374c0` returned nextest 100 with
`an ordinary panic must escape the crash helper: Crashed`.

The candidate replaces that identity with private-construction
`storage::FatalIo`, carrying actual `IoError` and context. All eight
Driver and five Log fatal sites use one raising function; unexpected
successful List completion remains an invariant failure. Best-effort
diagnostic writing preserves the old useful context without controlling
classification or installing an engine panic hook.

Measured mutations restored the shared prefix filter and one Log string
producer. Both compiled and failed their named assertions: ordinary text
was wrongly filtered to `None`, and an actual Log error lost its typed
payload. Exact sources were restored and all 26 fault tests passed.

The final catcher trace also found the host callback's broad catch
flattening genuine engine I/O into `FunctionFailed`. A public Database
callback regression on `292cf97d` injected one actual failed Write and
reproduced that exact conversion (nextest 100). Its narrow guard now
resumes the original typed payload while preserving ordinary provider
panic/shape/error behavior. This closes the same typed boundary rather
than introducing a new callback policy or public hook.

Independent reading review of the initial migration found no introduced
blocker; the lead traced and reviewed the small host-catch delta. The
worker-control query initially projected the public timestamp instead of
`Structure.where`; its exact text assertion caught that test error and
the complete catalog/row mapping justified correction. Final affected
checks and full integration are still required before a landing claim.
This remains the typed prerequisite, not the statement/commit search.

## 2026-10-31: typed fatal-I/O prerequisite landed and verified

Landed as `6c973e03e20a3248f8e91a0d36b98454a402457d`, based on
`f35ec560`. Fatal-I/O identity is a private-construction Rust payload
with original typed cause and context, raised at one storage boundary.
Shared job catches preserve ordinary text; the simulator helper accepts
only genuine typed I/O outside the existing crash lifecycle. The host
callback catch propagates that same payload and retains ordinary provider
failure behavior. Diagnostic strings no longer decide any of these cases.

The measured lookalike and host-flattening regressions, plus restored
prefix-filter and Log-string mutations, failed before their repairs.
Honest affected checks passed: 26 fault tests, 34 scrub, 11 commit and
17 host-function tests, including six counted ordinary provider failure
categories; affected clippy and format passed. The initial field-projection
failure remains recorded above, rather than being counted as a pass.

Final full `check` and the lead post-land full `check` both passed on
unchanged source: 3759 root tests, 48 campaign, 13 Loom and 39 Miri,
plus all configured static, documentation, grammar, policy, size and
tooling checks. Existing skips were root eight, campaign one, Loom
nineteen and Miri two. The independent reading review covered the initial
migration; the lead reviewed the small callback propagation delta.

Evidence limits: real failed I/O was exercised through storage producers
and a public host callback. Actual fatal I/O inside maintenance/reindex
worker catches remains supported by the shared gate tests and source
trace; it is not claimed as a separately executed fault case. Invalid
successful List completion and failed diagnostic emission also remain
source/type evidence. Direct best-effort diagnostics are captured by
tests and are not silenced by the panic-hook helper.

This prerequisite does not complete the matrix. Next add the bounded
public statement/commit operation-failure workload using the existing
search enumeration and typed fatal boundary. Start with Write/Sync
failures and two sequential atomic transactions; record acknowledgement
only after public commit returns, require acknowledged rows on reopen,
and permit only complete in-flight transactions where the contract allows.
Require actual fault-hit and outcome floors, including a failure after an
earlier acknowledged commit. Do not parse diagnostic text as fault identity.


## 2026-10-31: public commit search implementation

Track `fault-commit` starts from `ffd6c42d`, after the typed fatal prerequisite.
Use the existing operation enumeration for Write/Sync failures, two sequential
public transactions and an external acknowledgement ledger updated only after
commit returns. Reopen must preserve all acknowledged rows and allow only
complete in-flight transactions where the contract permits them. Require
actual fault-hit, operation-class and outcome floors, including a failure
after an earlier acknowledgement. No broad crash/rot or backend claim.
Targeted tests and one decisive mutation precede fresh review and one final
full check; compare landed inputs before reusing that receipt. The umbrella
stays open. This track is disjoint from the runtime text-fixture pilot.


## 2026-10-31: bounded public commit search landed

Landed as `9e349658d7596f9160690b154e3916c97d650050`. The existing
operation enumerator now supports typed fatal-I/O process outcomes without
accepting arbitrary panics. The public transaction workload records commit
attempts before invocation and acknowledgements after return, then reopens
and checks exact legal keyed-row prefixes between those counts. Both
statement-stage and commit-stage submissions were reached.

Default sampled six Write/Sync positions, quick three, and deep all sixteen
eligible positions of this one seeded baseline. The counts include failures
before the first new acknowledgement and after an earlier acknowledgement.
They do not establish coverage of other workloads, schedules, corruption
kinds or real backends. The umbrella remains open.

The independent reading review found an over-permissive acknowledgement-plus-one
oracle; the author corrected its upper bound to actual commit attempts.
An initial fixture rejection was corrected by declaring the self-key's
sequence generator while retaining every explicit key. Neither initial
failure is counted as fault coverage.

Three measured mutations failed as intended, and restored named runs passed:
a seed-only recovered image lost earlier acknowledged rows; synthetic ledger
inputs exposed a never-attempted recovered transaction; removal of the
production durability wait under deep exposed acknowledged-data loss at
position six (one acknowledged and one attempted transaction, recovered
prefix zero). The last result establishes production-path sensitivity for
this workload; the synthetic input is only oracle-bound sensitivity.

The author's final full `check` passed on unchanged tested source
`7cdd897b62d7b002abe8a88edb3d129cb6e6a5b6`: 3760 root tests, 48 campaign,
13 Loom and 39 Miri, plus all configured static, documentation, grammar,
policy, size and tooling checks. Configured skips were root eight, campaign
one, Loom nineteen and Miri two ignored. The check body took 751 seconds;
the new test took 0.916 seconds in that run. Landing changed only three
reviewed lead-workflow Markdown files relative to the tested tree. Code,
test inputs and configuration were identical, so the receipt was reused
without another Rust full check.

Rerun the focused search with `work/tools/suite -p adtb-sim --test commit
--profile deep --success-output final -E
"test(=public_commits_preserve_acknowledged_pairs_under_write_and_sync_failure)"`.
Typed-panic controls, existing backup failure-search callers, affected clippy,
formatting, house and size checks also passed. Next matrix work remains a
separately bounded workload or fault domain with explicit reached-case floors.

## 2026-11-02: maintenance and migration search

`test: search crashes and failures through Database in maintenance and migration`, crates/adtb-sim/tests/maintenance_faults.rs. Failed Write and Sync and crash images during vacuum, reindex and migration, and crash images on the commit workload. Default crash and failure positions are forced inside each action, with floors on faults in the action, data writes in it, and a migration absent in it. Deep passed at 73 vacuum, 134 reindex and 102 migrate failure positions. No defect found in unmutated code. Mutants of the waits in pager.rs `staged` and write/mod.rs commit die. The wait at catalog/migration.rs:156 dies only in `an_open_that_migrates_has_made_the_migration_durable_when_it_returns`, with a 64 MiB pool: under a small pool eviction forces the log. Full check exit 0; review's two reach blockers were fixed and confirmed closed. Default adds 4.35 s wall alone at 4 threads, deep 59.6 s.

Limits: one seed per workload, no concurrent writers. Review non-blockers left:

- maintenance_faults.rs `lead_ops` counts Write and Sync while crash marks number every op, so the skip is short and recovery data writes can satisfy the migrate crash data write floor. Filter marks before the skip.
- The start from `plain_open_cost` is not an exact bracket of `apply`: trailing open work counts as inside the action.
- Reindex has no floor showing a fault at or after the swap.
- A failed Sync drops nothing in `sim.files()`, so process death does not model dropped dirty pages; the crash search covers power loss.
- The filler lookup does not prove the index was used, and `index (rank)` is never queried.

Rows stay open: page rot, real backends, io_uring, scrub, verify, typed causes for `Internal` and `Corrupt`, create by publishing aside.

## 2026-11-02: page rot search

`test: search page rot through Database`, crates/adtb-sim/tests/maintenance_faults.rs. Flip, zeroed and torn on each page of a closed database, then open, scan, index lookup, insert and commit, `verify`, with a guard against hangs. Deep passed all 1872 cases, 624 pages by three kinds, 230 s. No panic, hang, wrong row or lost acknowledgement, no engine change. Default samples by page class, 27 pages, 4.6 s. With the checksum check disabled the search reports 13 defects, 2 wrong rows and 2 panics. Check exit 0, review found no blocker. Test only.

Limits, severe first, from the review:

- Floors count any read and any refusal, and `verify` proves the damage landed, but nothing proves a read crossed the damaged page. Point probes reach few leaves and no plan is asserted, so a read path that skips the checksum on other leaves could go unseen. Tie each read to the page damaged.
- No floor on an acknowledged commit read back after damage; only undamaged, header and visibility map flips reach it. Clean close only, no crash.
- `what` and `detail` of a refusal are unchecked, so it may name the wrong object.
- A leaked hung thread can slow later cases or panic into a later report: false failures only.

heap-read-foreign-page-panics stays open: rot fails the checksum or reads blank, so never reaches that path. Scrub is not searched: no public call starts a pass. Rows still open: real backends, io_uring, scrub, typed causes for `Internal` and `Corrupt`, create by publishing aside. Page rot is done with the limits above.

## 2026-11-02: rot reads tied to the damaged page

`test: tie page rot reads to the damaged page`, crates/adtb-sim/tests/maintenance_faults.rs. Test only, no engine defect found. Closes the first three limits above, for application objects.

`a_read_crossing_a_rotted_page_gives_the_written_rows_or_refuses_naming_it`: each case takes a fresh image and runs first a read that needs the damaged page: the full scan of a heap or large value page, an index lookup for each key the leaf's own bytes hold, on secondary and key indexes. The plan is asserted on the damaged database. A flip that answers is a defect. The refusal's `what` and the quarantine marks compare exactly. An open refusal is a defect. Deep: heap and store 56 cases per kind, leaves 294 per kind, all refused. Commits after damage elsewhere read back exact through scan, name probe and key probe, 82 per kind.

`a_commit_through_a_rotted_page_of_its_target_loses_no_row`: commit first, no read before it, damage in the `rows` heap, name leaf or key leaf. All refused the commit; no row lost.

Default adds 5.1 s against an asked 3 s, accepted. Deep 62 s. Check exit 0, suite 3562 passed. Mutants: read path skipping the checksum on chosen pages, `!=` lookup form, and a dropped refusal name each fail the search. One review and one delta; delta not separately reviewed.

Limits, severe first:

- The commit search has no case of a commit acknowledged on its own damaged target, so its lost row assertion has never fired. A checksum failure read as blank, scoped to `Heap::insert_pruning` and `BTree::insert_with`, survives: reads earlier in the statement refuse first. Widened to all of `Stmt::apply` it fails only by hang and `Fatal Storage Malformed`. `rows` has one page of each kind, so each commit row is one case.
- Inner B-tree pages have no crossing read. Floors group objects, so an object with no pages adds no case and no failure.
- Damage to a large value page refuses naming `the database` and marks `Database`, pinned by the test. Spec wants one constructor relation: quarantine-large-values.
- A read whose expected rows are empty counts as exact. Default takes one key per leaf and every profile one flip per page.
- The leaf parser restates node offsets. A change fails loudly.
- One seed, sim backend, one damaged page, clean close, no concurrent writers.

`check` run through `direnv exec MAIN` from a slot tested the main checkout and exited 0, found on this track's first check. `check` now refuses a `DEVENV_ROOT` that does not hold the working directory. The earlier fault-next check reported 3560 with main at 3558 and two new tests, so it did test its slot.

## 2026-11-02: foreign pages and real backends

heap-read-foreign-page-panics landed `fix: refuse a heap page another owner holds`: a valid page of another owner refuses and marks its holder, an entry naming an unused slot refuses its index, searched through `Database` by scan, lookup, large value and write by key. Its ticket stays open for leftovers.

real-backend-faults-engine landed `test: search Database under injected faults on both real backends`: `with_faults` injects every kind beneath io_uring with the pool's meaning, the fault contract and the crash cases run on both backends, and a `Database` commit workload under failures, torn and lost unforced writes keeps every acknowledged commit on both. Latent and corrupt reads are not in the `Database` search. Its ticket stays open for races between admission and completion and for file systems.

Rows still open: scrub, typed causes for `Internal` and `Corrupt`, create by publishing aside, and the test-audit-a floors in the report.

## 2026-11-02: test-audit-a floors

`test: floor quarantine refusals and verify findings by kind`. All four items held when run and are closed. A commit refused after a quarantine mark is reached by the renamed sweep `a_mark_lsn_follows_every_commit_kept_before_it_and_refuses_every_commit_after`: writers wait between statement and commit and the mark lands in that window; floor half the seeds, reached in every profile. The pager sweeps floor page and log findings by kind as a rate per seed run, set near half the deep rate, with quick and default on picked seeds; deep runs the full old ranges. `storage::OpenError::Io` carries an I/O failure opening the data file, which `NoHeader` used to stand for; the api maps it to `OpenError::Io`. Mutants: `admit_commit` always Ok, verify keeping only page findings, `Io` mapped back to `NoHeader`, each fails its test. Check exit 0, 3602 passed.

The deep run found a lost acknowledged commit no one reports: page-past-log-end-unreported, so `misdirected_and_lost_writes_are_reported_never_returned` fails under deep at seed 337 until it lands. Orphan findings are not floored: no sweep reaches one. create-io-failure-as-not-found holds the same I/O as damage on create.

Rows still open: scrub, typed causes for `Internal` and `Corrupt`, create by publishing aside, page-past-log-end-unreported.

## 2026-11-02: rulings on the last rows

None of the three changes anything a client observes, so none goes through the design space.

- Typed causes for `Internal` and `Corrupt`: the adopted ruling project/arch/errors/checker-guarantee/fail-statement fixes the client shape as `Internal { what: Text }`, and `OpenError::Corrupt { detail }` is the embedding shape. The public shapes stay. Inside the engine every site that builds one carries a typed cause, an enum per layer, and text is made only where the api builds the public error. No string stands for a cause a caller or test branches on; tests assert the cause, not the text.
- Scrub: no new call. The search starts scrub passes through the public `ScrubInterval` setting under simulated time, and floors pages scrub read and findings it filed, by kind, as the rot search does.
- Create by publishing aside: adopted. Create builds data file and log under reserved names beside the target, forces both and the directory, then publishes by rename in the order restore uses, and forces the directory again. A crash at any point leaves either no database or a whole one, so a retried create works with no file deleted by hand. A create finding its own reserved leftovers removes them first; any other existing file still refuses. The rename keeps open descriptors, so no reopen is paid. Searched through `Database::open` with `create`, crash at every op, as the host path.

## 2026-11-02: page past log end landed

page-past-log-end-unreported closed by `fix: report a page past the recovered log end`. Left from it: a log batch that passes its checksum but fails to parse reports its pages `Unlogged` on top of its own log finding, a second report of damage already reported. New defect class row: damage reported as nothing.

Rows still open: create by publishing aside, scrub through `ScrubInterval`, typed causes for `Internal` and `Corrupt`, each ruled above.

## 2026-11-03: create by publishing aside, abandoned

Track create-aside abandoned after one review and one delta review; its last commit is eb1db5f14b84 (change kmknnwos), kept as a reference, not landed. Both reviews found create deleting data it did not make, the same defect class twice: identity decided by name and shape.

- Round 1 built beside the target as `NAME.creating` and `NAME.creating-log` and removed leftovers by name, so creating `shop.adt` deleted a live database named `shop.adt.creating` and its log.
- Round 2 built inside `NAME.creating/` and removed it when it held exactly a file with a whole header and a log whose segments carry that header's file_id. A real database a user keeps at `NAME.creating/NAME` meets every part, and a probe deleted it with its committed row. On the host the removal takes no lock, and Linux unlinks a file another process holds a flock on.

What the next attempt needs, ruled now:
- Ownership by a mark no database has, written first: create makes `NAME.creating/` and writes a marker naming a fresh create id before anything else; removal requires the marker and that every built artifact carries the id. Without the marker nothing is removed. A user's directory without it refuses.
- Removal on the host only under the reserved file's lock, taken non-blocking; held means refuse.
- Keep from round 2: the runtime contract for a rename into the parent, forced after both directories sync (docs/design/runtime.md and the sim's disk.rs), roll-forward when `NAME-log` carries the build's id, create never deleting `NAME-log`, cleanup errors as `OpenError::Io`, create's I/O failures as `OpenError::Io`.
- Fix from the delta review: a failed reopen after a roll-forward on the host must not remove the published file (`remove_created` after `Held::take`); `publish_names`'s postcondition; an empty reserved directory left behind on a refusal; a first-block-only segment check.
- Stop line honest from the start: round 1 was +194 production against 120 and round 2 +229, so brief about 250 production and 300 test.

create-io-failure-as-not-found stays open: its fix was in the abandoned change.

## 2026-11-03: scrub row done

The scrub row is closed by `fix: leave a page the scrub cannot read for the next read to retry`, which carries the scrub search in crates/adtb-sim/tests/maintenance_faults.rs. Deep: 1878 rot cases, no defect. Leftovers on scrub-follow-ups.

Rows still open: create by publishing aside (abandoned once, ruled for the next attempt), typed causes for `Internal` and `Corrupt`.

## 2026-11-03: create by publishing aside landed

`fix: create a database aside and publish it last` (track create-aside-2). On main a crash at 3 of 1137 create points refused every retried create; after it none does. Ownership by a forced marker carrying the file id, removal only under the reserved file's lock and with every entry that id's, links refused on the host. The ownership search plants every shape the earlier attempts deleted, a zero-headed user file at the marker's name and a link inside the reserved directory, and compares every byte. Review blocker (a marker read by its first block only) fixed before landing. Leftovers on create-leftover-marker. Pager floors in `threads_sharing_a_pager_under_every_fault` reruled (picked-seed-floors).

Row still open: typed causes for `Internal` and `Corrupt`.

## 2026-11-03: typed causes, api layer landed

`refactor: carry a typed cause to every Internal and Corrupt`. Every non-test `Internal` and `Corrupt` is built in crates/adtb/src/api/cause.rs from a typed cause; public texts unchanged except Debug quoting dropped from some `Corrupt` details. Review blocker (a search test blind to shorthand, `String::from`, renames and code after a test module) fixed before landing. The engine layers still carry text into it: typed-causes-engine-layers.

Every row of the matrix is now landed or ticketed. Closing review next.
