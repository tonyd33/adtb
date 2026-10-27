---
created-at: 2026-10-27
status: open
kind: feature
priority: 823
summary: "bundle lease and notice periods are constants, an open supersedes every bundle it is not given, and in process no bundle is a live reader during a migration"
tags:
  - engine
---

# Feature: bundle follow-ups

## Report

- `LeaseGrace`, `DefaultLease` and `BlockingNoticePeriod` are constants, not read from `catalog::settings`.
- `catalog::bundles` does not mark how a bundle was registered, so an open supersedes every active bundle it is not given. A server registering bundles at run time needs the mark.
- Open records its bundles and supersedes the rest before migrating, so in process no bundle is a live reader while a migration runs. `sys::pending_drops`, `sys::jobs`, `BlockingMigration` notices and a migration report naming pending drops have nothing to report until run-time `apply migrations` or `register bundle "file"` exist.
- A `budget` clause in a bundle header is refused as unsupported.
- `explain B::q(args)` sees only bundles pinned at the current version.

## 2026-10-27: done

`DefaultLease` and `LeaseGrace` are read from `catalog::settings` at open and taken from each `set setting` as it commits (`crates/adtb/src/api/settings.rs`), and `RetryLimit` with them. Tests in `crates/adtb-sim/tests/bundles.rs`.

Still open: `BlockingNoticePeriod` has nothing to time until `BlockingMigration` notices exist, and the other items above.

## 2026-10-27: settings

`DefaultLease`, `LeaseGrace` and `RetryLimit` are read from `catalog::settings`, refreshed on each `set setting`. The rest stays open.
