---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 860
summary: "reverting a grant with grant option or with to on its own line builds a revoke that does not parse, and reverting a revoke drops the grant option"
closed: 2026-10-27
tags:
  - quality
  - lang
---

# Reverting a grant builds malformed revoke text

## Report

`inverse` builds the inverse of a grant or revoke by rewriting its text at `crates/adtb-lang/src/evolve/revert.rs:986-991`: `replacen("grant", "revoke", 1)` and `replacen(" to ", " from ", 1)`, and the reverse. Grants are now stored state (`crates/adtb-lang/src/evolve/grant.rs:41-66`), so the inverse is observable.

- `grant read identities to support with grant option` inverts to `revoke read identities from support with grant option`. The revoke rule has no `with grant option` (`parse/op.rs:668`), so the inverse does not parse.
- `grant read identities` / `  to support` has no `" to "`, so the inverse keeps `to` and is malformed.
- Reverting `revoke X from r` grants X back with `grant_option: false` even when `r` held the option, and grants X to `r` when `r` never held it.

No test reverts a grant or a revoke.

Repro to add to `crates/adtb-lang/src/evolve_tests.rs`: apply `grant read identities to support with grant option`, then `revert` it. Expected: the source holds no grant. Probably refused instead (found by reading, not run).

Fix: build the inverse from `before.grants`, restoring each `LogGrant` exactly, grant option included, and drop the text rewrite.

## 2026-10-27: done

The inverse of a grant or revoke is `Inverse::Grants`, built by `Cx::grants_back` (`crates/adtb-lang/src/evolve/revert.rs`) from the resolved grants: it applies the operator to the source before it and records, for each role and privilege whose hold changed, what the role held before, grant option and place included. `Source::hold` (`evolve/grant.rs`) sets each back, so a revert touches only what its operator changed. The text rewrite is gone. Tests in `crates/adtb-lang/src/evolve_tests.rs`: `revert_of_a_grant_removes_what_it_granted` (with grant option, `to` on its own line, several privileges), `revert_of_a_grant_keeps_what_the_role_held_before`, `revert_of_a_revoke_restores_the_grants_it_removed_exactly`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
