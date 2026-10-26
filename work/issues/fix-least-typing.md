---
created-at: 2026-10-25
status: open
kind: feature
priority: 290
summary: "fix binder types come only from base operands, not the least solution"
depends-on:
  - conformance-checker-findings
---

# Feature: least types for fix binders

## Report

`queries/11-fix:40` is refused: the checker types a `fix` binder only
from a `union` operand that does not read it (option
`spec/fix/typing/from-base`), but typing.md §2.2 gives each binder the
least element type at which every body checks. Waits for Tony to adopt
`spec/fix/typing/least` (point `impl1`). Then check the body with the
binder at an unknown element type solved from its `select` or the
expected type.
