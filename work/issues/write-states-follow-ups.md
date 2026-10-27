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
