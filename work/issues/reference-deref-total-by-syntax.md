---
created-at: 2026-11-16
status: open
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
