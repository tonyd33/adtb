---
created-at: 2026-11-17
status: open
kind: bug
priority: 550
summary: "the spec does not say what type an interval index's last element must have; the engine admits any keyable type and the index then behaves as a plain index"
tags:
  - spec
  - class-none
---

# Interval index element type unstated

## Report

Found by the index-keyable review (2026-11-17). `interval index (v)` is admitted over I8, Text, `()` and others and behaves as a plain index; interop.md lists interval indexes as an index kind but no part states what type the interval element must have (a span, a pair of bounds, an ordered scalar). Rule it in the spec through the spec-revision workflow, then enforce it in the checker beside `Ty::key_form`.
