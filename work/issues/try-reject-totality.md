---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 199
summary: totality analysis treats try as total though try of reject aborts
closed: 2026-10-25
---

# Bug: totality treats try as total

## Report

adtb-lang's totality analysis treats `try e` as total, but `reject`
is not a fault, so `try(reject "x")` aborts the statement. A function
the analysis calls total can then fail. Treat `try` as total only when
its operand cannot reject.

## 2026-10-25: resolved

`try e` is total only when no `reject` occurs under `e`.
