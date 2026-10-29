---
created-at: 2026-10-28
status: open
kind: feature
priority: 300
summary: "the checker recurses once per match arm in Checker::chain, so a match of 4000 arms overflows a 2 MiB thread at opt-level 0"
---

# Feature: bound the checker's stack by nesting

## Report

`Checker::chain` (crates/adtb-lang/src/check/pat.rs about line 970) recurses once per match arm. The reference-recursion track's wide tests (crates/adtb-reference/tests/wide.rs, 4000-arm scalar and or-pattern matches) overflow a 2 MiB thread inside the checker at `CARGO_PROFILE_TEST_OPT_LEVEL=0`; at the test profile's opt-level 1 they pass. Make the checker's stack grow only with nesting, as the parser's `MAX_DEPTH` bounds it, and look for other per-arm or per-item recursion in adtb-lang. Wait until version-translation lands (it owns adtb-lang).
