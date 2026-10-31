---
created-at: 2026-10-31
status: open
kind: bug
priority: 399
summary: "A pinned bundle's write reaches constructors past its pin: Internal on a legal upsert, a hidden row overwritten, a hidden value-sum field misread"
---

# Pinned writes past pin

## Report

## Report

All from reading in the review of staged-operators, except the first item, which was run. All on main before that track. Cause: a pinned translation types a value by the pin although its source ranges wider. Class: none listed fits exactly; closest is authority per entry path, since the gate in `Reads::facts` records an upsert only as a write.

- Upsert `old` on a Link row without the field: refused with `Internal: constructor 1 of the type has the field`, row unchanged. A legal call gets an `Internal`. Tests: `an_upsert_old_*` in crates/adtb-sim/tests/bundles.rs.
- `update old set new` by a pin overwrites a row of a hidden constructor.
- In a `hide new constructors` bundle, a stored value-sum field read through an ordinary scan (`x.tone.depth`) has the same shape: its source is wider than the pin.
- `Walk::field` takes the first owner, so a same-named field on a removed constructor refuses falsely.

Decide the rule first: does the gate count a conflict probe as a read, or does a pinned write refuse a row past its pin with a named error? Run the second and third items before fixing.
