---
created-at: 2026-10-26
status: open
kind: bug
priority: 799
summary: "a name outside a Key type that a later operator adds is accepted, since add_decl never type checks and only the final recheck runs"
---

# Migration names in order

## Report

## Report

`add type Pair = { left: Later }` before `add type Later`, or a `define fn` naming a relation added later, is accepted. Only `Key r` may point forward within a migration. `crates/adtb-lang/src/evolve/tests.rs` (or evolve_tests.rs) `a_name_outside_a_key_type_types_against_the_operators_before_it` fails until each operator types against the ones before it.
