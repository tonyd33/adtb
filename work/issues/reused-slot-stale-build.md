---
created-at: 2026-11-02
status: resolved
kind: bug
priority: 140
summary: A reused slot can build and test old code after it moves to another revision
closed: 2026-11-02
---

# Reused slot stale build

## Report

Track recorded-reader, on a slot moved to main by `slot`: source mtimes were older than rlibs of an earlier revision, 13:49 against 14:03, so cargo reused those rlibs and tested old code. `a_bundle_going_live_is_refused_at_other_host_code_and_admitted_at_the_same` failed on unchanged engine code with `the registration of web-1.0 is not refused`. Touching the crate roots cleared it.

Seen failing, not falsely passing, but the same state could pass a `check` on code other than the tree it names, and a landing relies on that check. Cause not established. `slot` and `land` only run jj, which writes changed files with the current time, so a build finishing after the move is the first thing to rule out: an agent's background run outliving its report, or a build using this slot's `CARGO_TARGET_DIR` from another tree.

Rule: a check names the tree it ran on. Enforcer wanted: reproduce first, then have `slot` or `check` refuse or invalidate build output newer than the last move of the working copy.

## 2026-11-02: cause found, closed

Cause: a mutant restored by `mv` of its backup, made by `perl -pi.keep`, took back its old mtime, so cargo kept the mutant's build. Reproduced on a free slot: after the `mv` cargo compiled nothing, after `touch` it compiled. The move of the slot played no part. The check of a6551d79 ran on that state, so it is not evidence for that commit; the later check over the same production code rebuilt from touched sources and passed. Class: verification reading a cache as if it were the tree.

`check` now touches every Rust source under crates/ and apps/ first, so it compiles the tree it names. Run on main c3910f87, exit 0, 3558 passed, recompiling `adtb-lang`. Nothing tests the touch itself. A targeted run has no such protection: code-agent.md bans mtime preserving restores. A file outside those directories that a crate includes is not touched.

## 2026-11-02: resolved
