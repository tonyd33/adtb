---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 401
summary: acknowledged work and failure outcomes are not searched through Database under every fault
branch: fault-open
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
