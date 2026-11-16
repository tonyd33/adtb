---
created-at: 2026-11-16
status: open
kind: bug
priority: 400
summary: a key selection listing a named set in parentheses is refused as not a constructor
tags:
  - lang
  - class-none
---

# Parenthesised named set refused

## Report

Found by the review of committed-schema-type part 2b-m-2 (2026-11-16). A key selection listing a named set inside parentheses, `Key regions.(Wild | Settled)`, is refused "`Settled` is not a constructor of `Region`", even when `Settled` is declared earlier. docs/spec/evolution.md §3.1 allows `Key r.(C | S)`. A valid schema is refused.

Fix direction: resolve each member of a parenthesised selection as a constructor or a named set of the relation, the union of their constructors; a test with a set declared before and after.
