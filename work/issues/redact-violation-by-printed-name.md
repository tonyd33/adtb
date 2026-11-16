---
created-at: 2026-11-15
status: open
kind: bug
priority: 420
summary: ad hoc redaction finds a violated denial or keyed relation by its printed name instead of the identity the write path raised it with
tags:
  - engine
  - class-identity-by-spelling
---

# Redact violation by printed name

## Report

Found by the review of structure-lang-identity step 11 part b (mmwzottl). `redact` in crates/adtb/src/api/server/adhoc.rs decides whether a role may see a violation's witnesses by finding the violated object from the reason's printed name: a constraint or check through `write::Database::denial_named(name)`, a key violation by matching the relation's `qualified` name. The write path raised the violation with its identity (`write::Violator`) and the reason drops it.

It holds today only by convention: the name searched is the `Denial.name` the write path printed, constraint and check names share a module's value namespace, and non-denial names contain a `.`. Nothing at the lookup site enforces it. A miss returns `None`, which redacts, so a failure hides a witness rather than leaking one.

Fix direction: carry the `Violator` on `write::Rejection::Violated` (one production construction site, eval.rs) and beside the api reason on the engine error, not in `RejectReason`; `redact` decides by it, and `denial_named` and the `qualified` match go. A test that a witness is shown or redacted for each violation kind by identity.

Defect class: Catalog rows and identity by position or spelling.
