---
created-at: 2026-10-26
status: open
kind: feature
priority: 86
summary: "codemap and house-check live in the lead's scratchpad, codemap skips tests, and house-check flags Rust derefs and spec section signs"
tags:
  - tooling
  - devex
---

# Feature: agent orientation and house-rule tools in the repo

## Report

Every track brief points at two scripts in the lead's session scratchpad, `bin/codemap` and `bin/house-check`. Ten of eleven 2026-10-26 tracks ran codemap and nine ran house-check, but the scripts are unversioned and disappear with the session.

codemap lists only `src/`. Test binaries are where agents look next: eadhoc and app ran `ls crates/adtb/tests` (it does not exist; engine integration tests are in `crates/adtb-sim/tests`), sound and speed listed every crate's `tests/` by hand. `codemap adtb-testkit fixture.rs` looks for `fixture.rs.rs` and prints `0L` (sound).

house-check greps every added line for `**`, `§` and `//`, so it flags Rust double dereferences (`(**row).clone()`, strength 3 lines, eadhoc), section signs in spec and design prose (evo, qual4), and agents read each hit to dismiss it.

Fix: move both into `work/tools/` as devenv scripts. codemap gains a `tests` view listing each crate's test binaries with their test counts, and accepts a module given as a file name. house-check checks bold, section signs and rule ids only inside Rust comments and checks em dashes everywhere.
