---
created-at: 2026-11-12
status: open
kind: bug
priority: 360
summary: "reverting a drop of a relation restores grants on its sets but loses a grant on the whole relation"
tags:
  - class-copied-rules
---

# Reverting a dropped relation loses its whole-relation grant

## Report

Setup: `add type Y { id: Key ys } = Y1 { a: Text } | Y2 { b: Text }`, `add relation ys : Y` with `key id generate sequence`, `unique (a) in Y1`, `default Y1.a = "a"`, `computed c: Text = "c"`; `add set YS of ys = Y1 | Y2`, `add constructor Y.Y3`, `deprecate ys.Y2`, `grant read ys to support`, `grant insert ys.YS to support`. Then `drop set YS of ys` / `drop relation ys` / `drop type Y`, and revert: `grant read ys to support` is gone, `grant insert ys.YS` comes back; the value states of Y1, Y2, Y3 are lost too (p4 of structure-lang-identity). Found in the review of the step 5a law search; no case dropped a granted relation before.

Direction: the inverse by id restores every grant naming the dropped object (structure-lang-identity step 5). The law search's `KNOWN` row for this case goes.

Defect class: Wildcards and copied rules.
