---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 287
summary: "semantics §5.1 evaluates fields as written but the kernel sorts them by name, so which fault is reported can differ"
closed: 2026-10-25
tags:
  - spec
---

# Feature: fault choice among the fields of one value

## Report

Found by the reference interpreter. The kernel sorts record and
constructor fields by name; `docs/spec/semantics.md` §5.1 evaluates
them as written. In `{ b: 1 / z, a: x + 1 }` the reported fault can be
the division or the overflow depending on which order is followed.

Ruling: which fault is reported among the fields of one record or
constructor value is a declared freedom, like the existing one of
semantics §1.1. It goes through the design space as a question, then
into the spec.

## 2026-10-25: resolved

In the spec with `docs: record impl4 rulings`.

Kernel side: `docs/design/kernel.md` §3 drops its field-order TODO; a consumer may evaluate fields in name order.
