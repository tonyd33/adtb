---
created-at: 2026-11-11
status: open
kind: bug
priority: 470
summary: "a call whose statements succeed but whose results fail a checker guarantee ends its transaction, where the spec says Internal leaves it open"
tags:
  - class-not-atomic
---

# A call's results failure ends the transaction against the spec

## Report

## 2026-11-11

In api/statements.rs the `Ok` arm of `Engine::resumable` maps a failure of `p.results(..)` with `failed(e, true)`, so it ends the transaction. Its only error is `Broken::NotASet` (api/text.rs), a checker guarantee, and catalog.md §4 (`Internal`) and transactions.md say an `Internal` failure leaves the transaction open. At that point every statement of the call has succeeded and recorded its effects, so leaving the transaction open would keep a failed call's effects: ending it is what keeps the call atomic today. Unchanged by structure-transactions step 11, whose review found the conflict.

Direction: take the call's statements back before reporting a results failure (so the transaction can stay open with nothing of the call in it, as the spec says), or check the results' shape before any effect is recorded; a test with a checker-guarantee results failure asserting the reply and that the transaction stays open with none of the call's writes.

Defect class: Not atomic: an operation that must be all-or-nothing can stop partway.
