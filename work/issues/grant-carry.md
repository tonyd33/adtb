---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 825
summary: "structural operators and their inverses do not carry log grants, and a grant without a field list does not freeze its fields at apply"
branch: grant-carry
tags:
  - lang
---

# Feature: grant carry

## Report

First slice of the canonical relation-grant provider in roles-and-grants (provider order step 1, language part). `Source::grants` (crates/adtb-lang/src/evolve/grant.rs) holds `LogGrant`s by catalog id, but no structural operator touches them, and `Granted::Read/Update(_, None)` keeps the omitted field list as syntax.

security.md 3.3 and 3.6 state the behaviour:

- A `read` or `update` grant without a field list covers the fields its constructors have when it is applied; a later field needs a new grant. A grant on the relation variable alone, without a list, also covers every field of each constructor it gains. A grant on a named set covers a constructor that joins the set.
- Rename: every grant follows (catalog ids are stable, so this should already hold; test it).
- Split: each new constructor receives the old constructor's grants on the fields its routing carries whole, under their new names; a computed field is not covered.
- Merge: the merged constructor receives a grant only where every source constructor was granted the same fields under it; otherwise the migration is refused unless grant operators in the same migration state the merged constructor's grants.
- Nest, unnest: grants follow the fields as moved.
- Remove: grants on the removed constructor or field are dropped.
- Every inverse restores the grants exactly (the operator set is closed under inverses; `revert/mod.rs` `grants_back` handles only grant operators today).

Out of scope: catalog rows in `catalog::grants`, grant row ids, missing-role refusal, dormancy, live bundle rechecks. Those are the next slice and consume this one's `Source::grants`.

## Fix

Freeze the field set at apply in the stored grant, carry in each structural operator and its inverse, refuse the merge case, and test each row of the 3.6 table plus the freezing rules and an inverse round trip for each. Also carries the first item of migration-checker-follow-ups.
