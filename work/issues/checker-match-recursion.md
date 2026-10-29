---
created-at: 2026-10-28
status: resolved
kind: feature
priority: 300
summary: "the checker recurses once per match arm in Checker::chain, so a match of 4000 arms overflows a 2 MiB thread at opt-level 0"
closed: 2026-10-29
---

# Feature: bound the checker's stack by nesting

## Report

`Checker::chain` (crates/adtb-lang/src/check/pat.rs about line 970) recurses once per match arm. The reference-recursion track's wide tests (crates/adtb-reference/tests/wide.rs, 4000-arm scalar and or-pattern matches) overflow a 2 MiB thread inside the checker at `CARGO_PROFILE_TEST_OPT_LEVEL=0`; at the test profile's opt-level 1 they pass. Make the checker's stack grow only with nesting, as the parser's `MAX_DEPTH` bounds it, and look for other per-arm or per-item recursion in adtb-lang. Wait until version-translation lands (it owns adtb-lang).

## 2026-10-29: dispatched

A fresh writer owns this in workspace checker-recursion. version-translation has not landed, but it changes only translate/, schema.rs and lib.rs in adtb-lang, so the writer stays out of those files and lists any recursion there for a ticket. Pattern: the reference's tail-position loops. Gates: suite over adtb-lang, adtb-reference and adtb-testkit, wide.rs at opt-level 0, clippy, fmt, house-check, size-check. Local brief: work/briefs/brief-checker-recursion.md.

## 2026-10-29: landed

Landed as 761788fb after a clear review; the main check passed on it. `Checker::chain` loops over arms, `wild_rule` and `useful` walk constructor fields with an explicit stack in the same depth-first order, and the normaliser's `spine` folds `If`/`Let` tails into one loop with the memo lookups where the old calls were. Kernel output is unchanged. crates/adtb-reference/tests/wide.rs passes at opt-level 0 (5/5; 2/5 overflowed before). Left: match-failure-duplication (a guarded arm that also tests a literal or field copies its failure continuation) and checker-wide-types (thousands of parameters or constructors).

## 2026-10-29: resolved
