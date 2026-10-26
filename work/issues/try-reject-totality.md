---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 199
summary: totality analysis treats try as total though try of reject aborts
branch: adtb-chk
---

# Bug: totality treats try as total

## Report

adtb-lang's totality analysis treats `try e` as total, but `reject`
is not a fault, so `try(reject "x")` aborts the statement. A function
the analysis calls total can then fail. Treat `try` as total only when
its operand cannot reject.
