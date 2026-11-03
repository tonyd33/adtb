---
created-at: 2026-10-29
status: open
kind: feature
priority: 425
summary: "Database::open prefers io_uring while with_faults panics on it, and no Database workload runs under faults on any real backend"
---

# Feature: Real backend faults engine

## Request

Slice of milestone 9; scope, tests and acceptance in work/briefs/review-m9-audit.md, S15. Test oracle is verify-online.

## Proposal

## Open questions

## 2026-11-02: faults beneath the ring, in review

Workspace real-faults. The injector splits into admission and completion, which the pool calls under one lock and io_uring calls around the device; the fault contract cases run on both backends; a `Database` commit workload under `fail_p` and `torn_p` crashes and reopens on both. An injected write or Sync failure inside a commit unwinds `FatalIo` by the adopted fsync ruling, and the test takes it as the crash. No lost acknowledged commit found. Review found no blocker; one delta asked: `stop` drains the ring, floors on torn and lost unforced writes at a crash, the pool-only crash tests on both backends, stale docs, `ADTB_URING=absent` documented.

Left, severe first, all from admission and completion no longer sharing one lock under the ring:

- `crash` taking the lock after the device finished a `Remove` but before its completion leaves the file's writes pending, and `lose` panics reopening it. Open-create and CreateDir in the same window survive the crash outside the prefix rule for creations and removals.
- The `corrupt_p` write-back writes the whole stale read buffer, so a write landing between the read's device completion and its completion is reverted, unrecorded, and a later Sync keeps the revert.
- A Sync takes writes admitted but still in flight as forced; an engine issuing Sync before its writes complete would be hidden.
- An injected Sync failure runs `lose` while writes it covers may still be in flight; they can land after the restore.
- Two overlapping Syncs where the first fails on the device restore bases older than the second's forced writes. Unreachable under injection.
- Latent and corrupt kinds are not in the `Database` search: the injector does not report the damage it causes, so `verify`'s answer has nothing to compare with.

## 2026-11-02: landed

`test: search Database under injected faults on both real backends`. The delta landed as asked: dropping a runtime joins its threads, so every admitted op completes first, pinned by a drain test per backend; `OsRuntime::crash` returns `Loss { lost, torn }` and the `Database` case floors both per backend, reaching them with a 256 KiB pool and 1500 byte names; the two Pager crash cases and the `Database` case run on both backends from one list. Check exit 0, 3601 passed. Production +302 -150 and a delta of +75 -31, over the stop lines on added lines, accepted: most is the injector split moving existing bodies.

Stays open for the leftovers above and S15b, file systems under faults. S15b waits for a plan that uses no mount, FUSE or device mapper, nothing outside the test's own processes.
