---
created-at: 2026-11-16
status: resolved
closed: 2026-11-16
kind: bug
priority: 175
summary: "the reference panics on a pattern over the regex crate's compiled size limit, which the engine refuses"
tags:
  - lang
  - class-failure-flattened
---

# The reference panics on a pattern over the compiled size limit

Reproduced 2026-11-16 by the audit-candidates triage (receipt-audit-candidates-r1, local) on main 2efdbea4. `matches("x", "(a{1000}){1000}")` panics the reference ("Compiled regex exceeds size limit", adtb-reference/src/unicode.rs), while the engine refuses it as Invalid ("does not run in this engine yet"). prelude.md's `matches` states no limit on a compiled pattern.

Fix direction: the checker compiles a literal pattern and refuses one that does not compile, with one stated reason, so both sides refuse alike; prelude.md states the rule.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-16, resolved

Landed as lzvwznnq 2a20085f (2 rounds), track checker-one-gate: one `adtb_lang::pattern::Pattern::new` compiles every pattern for the checker, the engine and the reference, refusing at check a pattern over the size limit and any flag outside `i`, `m`, `s`, `U` with our own reasons; clippy `disallowed-methods` and feature-check keep it the only compile. prelude.md states that a pattern whose compiled form exceeds the implementation limit is an invalid `re`. Main's full check: 3859 of 3860, one timeout under two-track load (laws-revert-test-near-timeout), which passes alone in 29.5 s.
