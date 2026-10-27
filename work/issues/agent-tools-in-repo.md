---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 86
summary: "codemap and house-check live in the lead's scratchpad, codemap skips tests, and house-check flags Rust derefs and spec section signs"
closed: 2026-10-26
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

## 2026-10-26: done

`codemap` and `house-check` live in `work/tools/` as devenv scripts. `codemap` with no argument lists the crates; `codemap --tests [CRATE]` lists each crate's unit test count and each `--test` binary with its test count; a module may be given as a file (`fixture.rs`, `heap/page.rs`) and an unknown one is an error. `house-check [DIR [JJ-DIFF-ARGS]]` prints `FILE:LINE: RULE: TEXT`: an em dash anywhere; bold, a section sign or a rule id only inside a Rust comment (`UTF-8` exempt); and every added non-doc Rust comment, labelled as one to keep only for an invariant. It exits 1 when it prints. Both default to the workspace holding the script.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the devex track.
