---
created-at: 2026-10-27
status: open
kind: feature
priority: 855
summary: "pending drops have no sys tables, notices or migration report, an arm cannot name a pending constraint, and a server must bind the program that keeps pending drops"
tags:
  - engine
---

# Feature: write state follow-ups

## Report

- `sys::pending_drops` and `sys::jobs` have no rows, there are no `BlockingMigration` notices, and `migrate` returns `()`, so a successful migration does not name the drops it left pending.
- Readers are stated by `Catalog::register`; deriving them from registered bundles and the supersession of `evolution.md` section 8.4 waits on `bundles`.
- An `arm` clause naming a constraint whose drop is pending is refused, because the version's source no longer has it, so a constructor that the constraint matches on cannot be added until the drop takes effect.
- The write path an open builds uses the version's program. Embedded, nothing is pending at open; a server that keeps drops pending must bind the program that keeps them.
- `crates/adtb-sim/tests/write_states.rs` takes about 17 s of wall time; most of its 38 tests take 1 to 3 s.

## 2026-10-27: done

Bundles now drive readers first in process: a bundle registering at a pin before a constructor enabled since is refused `BundleStale` (or `BundleExpired` when its lease ran out), an open whose older bundle would read one is refused `DowngradeRefused`, and an open supersedes the bundles of earlier opens before it migrates. `Read::Relation` now means ranging only: a reader that reads rows whole states their fields, so ranging over a relation no longer blocks `enable T.C` for a value-level sum (the write_states case `rows` moved to the cases that do not block).

Open: in process no bundle is a live reader while a migration runs, since open records its bundles unchecked and supersedes the others first, so `sys::pending_drops`, `sys::jobs` rows, `BlockingMigration` notices and a migration report naming pending drops still have nothing to report; they need `apply migrations` at run time or the server. The other items of this ticket are untouched.

## 2026-10-27: bundles landed

Readers are now derived from registered bundles. The sys tables, notices and migration report items moved to `bundles-follow-ups`.
