---
created-at: 2026-10-25
status: open
kind: bug
priority: 241
summary: "the reference prints a Span as Span { start, end } and cannot read span(a, b), against interop.md §1.3"
tags:
  - reference
---

# Bug: The reference's Span text form

## Report

`docs/spec/interop.md` §1.3 says a `Span` prints and reads as
`span(a, b)` (ruled in `spec2`). The reference interpreter still
prints `Span { start: 1, end: 2 }` and refuses `span(3, 7)`, so 22
cases of `every_primitive_agrees_with_the_reference` fail.

The reference also reads and prints dates and times past the `Instant`
range (`+292278-12-31`), which looks like i64 overflow; `model.md`
bounds them. Refuse them.
