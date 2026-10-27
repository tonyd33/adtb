---
created-at: 2026-10-27
status: open
kind: bug
priority: 864
summary: "copied_from discards each replayed operator's error and keeps replaying on a half-applied source"
tags: [quality, lang]
---

# copied_from ignores a failed replay and marks later operators against the wrong source

## Report

`copied_from` at `crates/adtb-lang/src/evolve/mod.rs:662-670` replays operators with `if replay.apply_noting(..).is_err() { continue; }` and nothing after it in the loop body, so each error is discarded as a `let _ =` would. After one failure the later operators replay on a half-applied source and their marks can point at the wrong text.

Fix: `migrate` already applied these operators, so a failure breaks that invariant: `expect` naming it, or stop the replay with `break`.
