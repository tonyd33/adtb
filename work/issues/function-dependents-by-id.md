---
created-at: 2026-11-16
status: resolved
closed: 2026-11-17
kind: feature
priority: 118
summary: "a function redefinition finds the indexes and constraints that use it by text, so stale index keys and unchecked constraints survive a body change"
tags:
  - engine
  - lang
  - class-identity-by-spelling
---

# Feature: Function dependents by id

## Request

Mechanism for index-keys-stale-after-redefine-fn (a committed unique violated after `redefine fn`) and redefine-fn-skips-constraint-revalidation, from the cause review of 2026-11-16. A migration decides what a function change touches by the text of the index element or constraint that names the function: crates/adtb/src/catalog/change.rs:88-118 `shapes` compares element text, so a body change rewrites nothing and revalidates nothing.

## Proposal

One dependency relation from each index element, constraint, check and view to the functions it reaches, by function id, transitively through the functions they call, computed from the checked schema. A `redefine fn` whose body changes marks every dependent index for rebuild and every dependent constraint and check for validation. A searching test redefines a function under each kind of dependent, direct and through another function, and asserts the rebuilt keys and the validation outcome whole.

Exit: both tickets above resolved by it; no rewrite decision reads element text.

## Open questions

## Resolution 2026-11-17

Landed 2026-11-17 as `fix: rebuild and validate the dependents of a redefined function by id`. `adtb_lang::depends` (`bodies`, `reached`, `changed`) is the one input; bodies compare under the mapping of positions to catalog ids, so a shift changes nothing; domains are validated over stored values. No rewrite or revalidation decision reads text; `IndexSpec.fields` removed. Search: crates/adtb/src/catalog/fn_dependents_tests.rs. Filed: wasm-identity-by-name, aggregate-in-scalar-expression, evolution-redefine-dependents-unstated.
