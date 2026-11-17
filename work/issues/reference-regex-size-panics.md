---
created-at: 2026-11-16
status: open
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
