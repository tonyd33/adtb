---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 861
summary: "a revoke whose field list reorders the grant's removes nothing and says nothing"
closed: 2026-10-27
tags:
  - quality
  - lang
---

# A revoke with the grant's fields in another order leaves the grant held

## Report

`granted` at `crates/adtb-lang/src/evolve/grant.rs:127-145` keeps field ids in written order (`out.extend(found)`), and the revoke at `:61-64` matches on `Granted` equality. Constructor sets are sorted (`relation_sel` collects a BTreeSet); field lists are not.

Repro: `grant read identities (user, email) to s`, then `revoke read identities (email, user) from s`. Actual: the grant is still held and the revoke succeeds. Expected: the grant is gone, since a revoke takes the same forms as the grant.

Fix: sort and dedup the field ids in `granted`.

## 2026-10-27: done

`granted` sorts and dedups the field ids of a field list, so a grant and a revoke that name the same fields in another order resolve to the same privilege. Test: `evolve::tests::a_revoke_takes_the_grant_fields_in_any_order`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
