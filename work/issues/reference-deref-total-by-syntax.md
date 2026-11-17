---
created-at: 2026-11-16
status: resolved
closed: 2026-11-16
kind: bug
priority: 170
summary: "a deref through a constructor literal's reference field is classed total by syntax, so the normaliser drops a deref the reference faults on"
tags:
  - lang
  - class-none
---

# A deref through a constructor literal's reference is classed total by syntax

Reproduced 2026-11-16 by the audit-candidates triage (receipt-audit-candidates-r1, local) on main 2efdbea4. check/expr.rs classes `Deref.reference` by the syntax of the base, so a reference field read from a constructor literal holding a fabricated key (`customers.key(99)`) is classed total. The normaliser then drops a deref that is not used: `{ a: (Payment.Card { .., customer: customers.key(99), .. }).customer.name, b: 1 }.b` gives `[1]` on the engine, while the reference faults `NoSuchRow`. A control without the constructor literal agrees.

Fix direction: class a deref total only when the base carries a row it was read from (`Out.rows: Some(_)`); check what a `with` update carries (check/record.rs). A conformance case of the probe.

Defect class: none.

## 2026-11-16, resolved

Landed as ywltryxv bf307621 (2 rounds), full check 0 (3861 tests). Every checked value carries a private `check::Provenance` (Snapshot, Proposed, Reference, Built) set where it is made and inherited by generator, aggregate, match and group binders, computed `self` and transaction `let`; a deref is total only over a `Reference` key, a reference field read from a snapshot row. The `proposed`/`snapshot` variable sets and the `mentions_proposed` term scan are gone. The review found the same hole open through binders over proposed rows (predating this ticket); fixed here. conformance/queries/15-unused-deref.test pairs each kind with a key naming no row and one naming a row, its cases bound to the enum. semantics.md §3.4 now says a reference field of a row of the snapshot. Known gap: the provenance is fixed at check, so a function's row parameter stays partial after inlining; no plan class or admission changed.
