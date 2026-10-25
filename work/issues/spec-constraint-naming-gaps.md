---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 290
summary: "the spec does not say how a unique without a constructor scope is named, what BadInput reports for several sources, or what RejectedRow.source holds"
branch: adtb-spec
tags:
  - spec
---

# Bug: constraint names and rejected-row fields are underspecified

## Report

Found by the reference interpreter's writes. Rulings, each to go
through the design space before the spec changes:

- a unique with no constructor scope is named without an `in` clause;
- the `docs/spec/constraints.md` example `unique(provider, sub) in
  OAuth` must write `in (Google | Apple)`, its constructors;
- `BadInput` source and line are defined only for one source relation;
  otherwise they are `""` and `0`, and the spec says so;
- `RejectedRow.source` is the unqualified relation name.
