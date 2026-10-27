---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "every bundle call takes the engine-wide deployed lock, and a due renewal holds it across a durable commit whose failure refuses the call"
tags:
  - quality
  - engine
---

# Every bundle call waits on one lock held across a lease renewal's durable commit

## Report

`Engine::bind` (`crates/adtb/src/api/bundle.rs:682-698`) locks `deployed`, one mutex over every bundle's row, and keeps it while `replace_bundle` (`bundle.rs:754`) runs `swap_row`, a whole transaction with a durable commit. Every `Registered::call` takes the lock through `entry` (`crates/adtb/src/api/registered.rs:695`), as does `Database::registered` (`registered.rs:600`); `retire` and `supersede` hold it the same way. So:

- every call of every bundle waits behind one renewal's WAL flush;
- a renewal write that fails (serialization failure, `IoError::Full`) refuses the user's call though only the lease record failed;
- a call made from inside an open `Transaction` starts a second transaction on the same thread;
- `entry` renews the lease before refusing a call as `Untranslatable` for a bundle pinned at an older version (`registered.rs:695-703`).

Fix: decide the refusal and whether a renewal is due under the lock, write the renewal outside it with a single writer, and check the version before renewing. Either do not fail the call when only the renewal fails, or state in `docs/design/` that it does.
