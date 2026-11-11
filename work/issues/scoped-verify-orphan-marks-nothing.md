---
created-at: 2026-11-11
status: open
kind: bug
priority: 430
summary: "a verify scoped to one relation whose constraint meets an orphan entry in another relation's index reports a Structure finding and marks nothing"
tags:
  - class-damage-unreported
---

# A scoped verify's constraint meeting an orphan entry marks nothing

## Report

## 2026-11-11

Since structure-transactions step 8, a verify's constraint run that meets an orphan index entry (`exec::Error::OrphanEntry`) answers with a `Structure` finding instead of failing. When the verify is scoped to relation R and the constraint reads R but probes another relation's key index holding the orphan, the index check does not cover that index, so no `Orphan` finding is made and no mark is set. A query meeting the same orphan goes through `fail`, whose `OrphanEntry` arm marks the index. Main replied `Rejected Internal` here, so nothing is lost against main; the two paths disagree on the same evidence.

Direction: have the constraint run's `OrphanEntry` mark the index it names (as `fail` does), or add that index to the verify's index check when a constraint in scope probes it; a test with a scoped verify and an orphan in an out-of-scope key index.

Defect class: Damage reported as nothing: evidence of loss falls between two checks, each excusing it as the other's case.
