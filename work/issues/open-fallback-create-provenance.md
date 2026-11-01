---
created-at: 2026-10-31
status: resolved
kind: bug
priority: 560
summary: a create racing an unlink reports its newly created file as preexisting and leaves it after refusal
closed: 2026-10-31
tags:
  - quality
  - engine
---

# Open fallback create provenance

## Report

Read-only candidate from the quality review through `8538911c095c`, not yet
reproduced. `crates/adtb/src/api/file.rs:28-40` first tries `create_new`
when creating, then falls back after `AlreadyExists` to an open with
`create(true)`, whose result always carries `created = false`.

A concurrent opener holding the existing file's lock can unlink that
file between the failed exclusive create and the fallback open. The
fallback then creates the replacement itself but reports it as
preexisting. If the migration or registration subsequently refuses,
`crates/adtb/src/api/mod.rs:611-616` skips `remove_created` and leaves the
newly created file and its claimed log. This contradicts the cleanup
postcondition in `docs/design/server.md` section 4.1, even when every
unlinker holds the old file's lock and all cleanup operations would
succeed.

The existing `Held::take_opened` hook runs after `open_file` returns,
so `a_path_unlinked_before_the_lock_is_created_again` does not reach
this window. A regression must remove the preexisting path after the
first `AlreadyExists` and before the fallback, observe creation
provenance, then drive a refused `Database::open` and assert that its
new file and log are absent while an unrelated existing file is intact.

Fix direction: opening an existing file must not also create one. If
the noncreating open finds the name gone, retry the exclusive-create
protocol and retain the result of the operation that actually created
the inode. Class: multi-step change not atomic under failure or race;
the ownership fact must come from the creating operation. Include this
host filesystem window in fault-matrix's public-open coverage.

## 2026-10-31: public-open reproduction

The candidate is reproduced through actual `Database::open` on base
`38f07e0e`, with only a private test callback and regressions added
(`6f04ee6ebb4d67527d0ca3db8a04da96ba150ed9`). A cooperating holder
unlinks the original inode between exclusive-create collision and the
fallback. A missing `slugify` binding required by a constraint then
returns exact `HostFnMissing { name: "slugify" }` after the pager has
claimed its log. Both the replacement file and log incorrectly survive:
expected retention `(false, false)`, observed `(true, true)`. The named
window counter had reached one; missing/existing/replaced controls ran
before the failing unlinked case. Nextest returned 100.

Reproducer: `work/tools/suite -p adtb --lib
a_refused_open_keeps_only_files_it_found_across_creation_races` in
`fault-open`. The proposed repair uses a noncreating fallback and typed
retry through the existing bounded lock-acquisition loop. It introduces
no second retry loop, budget or public API. Fixed-source checks,
preservation mutation, independent review and complete integration are
still pending; this report does not yet close the ticket.

## 2026-10-31: bounded protocol fix landed

Landed as `3177391253a874d60614f5ba87a2980cd12d8d13`, based on
`e1716e22`. The fallback open cannot create a file. A vanished pathname
returns a typed retry to the existing bounded acquisition loop, and
only successful exclusive creation supplies `created = true`. This
repairs creation provenance for the protocol rather than special-casing
one refusal or filename. Locking and inode validation remain in their
existing boundary; no second retry loop, public API or dependency was
introduced.

The four-case test calls actual `Database::open` and checks missing,
existing, replaced and unlinked paths, exact window counts, exact
post-log-claim refusal, owned file/log cleanup, retained inode and an
unrelated sentinel. A separate strengthened real-file test acknowledges
`ann`, receives exact version-2 `SchemaRefused` for an invalid migration,
then reopens and returns exactly `ann`. An initial expected `Invalid`
category was corrected only after tracing the catalog/API wrappers.

The old protocol failed as recorded above. A second mutation that
misclassified fallback-opened files as created also failed: the existing
case expected both file and log retained, but both were removed. Source
was restored and the honest checks passed. These measure both directions
of ownership; they do not exhaust arbitrary filesystem schedules.

Independent source review found no introduced blocker. Final full
`check` passed on unchanged `31773912`: 3755 root tests, 48 campaign,
13 Loom and 39 Miri, plus configured static, documentation, grammar,
policy, size and tooling checks. Root skipped eight, campaign one,
Loom nineteen and Miri two. Targeted checks also passed: 52 API unit
tests, all 22 real-file tests, missing-host fixture, clippy and format.
The lead post-land full `check` also passed on an identical tree, with
the same test counts and all configured checks. The file/log ownership
race covered by this ticket is resolved.

Scope limit: the callback covers the host syscalls before the runtime
exists. Runtime operation failure/crash/rot searches remain in
fault-matrix. The preexisting acquisition failure before a Held is
returned is separately tracked in open-acquisition-create-cleanup.

## 2026-10-31: resolved

Resolved by `3177391253a874d60614f5ba87a2980cd12d8d13`. The preceding
landed receipt records the measured regressions, review, final full
check and lead post-land full check. Broader searches remain in
fault-matrix; acquisition failure before a Held is returned remains
separately tracked.
