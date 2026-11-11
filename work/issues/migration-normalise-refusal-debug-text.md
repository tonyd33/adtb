---
created-at: 2026-11-11
status: open
kind: bug
priority: 500
summary: "a migration whose schema does not normalise is refused with the normaliser error's Debug text as the reason"
tags:
  - class-failure-flattened
---

# A migration refusal carries a Debug-formatted normaliser error

## Report

## 2026-11-11

`Catalog::admits` in catalog/migration.rs turns a `norm::program` failure into `Error::Refused(vec![format!("the schema does not normalise: {e:?}")])`, so the client's refusal reason is the normaliser error's `Debug` text: unstable across refactors, unreadable, and not a typed reason a client can match. Found by structure-transactions step 8, which removed the other Debug-built error payloads before the api boundary and left this one because typing it changes a client-visible refusal.

Direction: decide whether a schema that type-checks can fail to normalise at all (if not, this is an engine invariant and a panic naming it, per fatal-split); if it can, give the refusal a typed reason with the failing definition's name and the normaliser's cause, and state it where migration refusals are specified.

Defect class: Damage or failure gives a panic, hang or flattened error.
