---
created-at: 2026-11-03
status: resolved
closed: 2026-11-03
kind: bug
priority: 140
summary: "translation never expands a revert into the operators it applies, so a revert of move field or rekey applies under a live pin and fails at call time, and a revert's fill is not seen"
---

# Translation does not expand a revert

## Report

Found by the translate audit. One mechanism: a `revert` is never expanded into the inverse operators it applies before `uncrossed`, the blame functions and `fill` look at the migration.

- crates/adtb-lang/src/translate/mod.rs:370-385 `uncrossed` has `_ => false` over `Resolved`, so `Resolved::Revert` passes unjudged. Revert of `move field Identity.EmailPass.email to Identity` with a pinned bundle reading `i.email`: pin `["a@x", "g@x"]`; after, the migration applies and the call fails `Internal "constructor 1 of the type has the field"`, where 7.2 refuses at apply. Revert of `rekey prices key (sku, n)` with a bundle returning `Set (Key prices)`: pin `prices.key(n: 1, sku: "a")`; after, the result fails to print (`a value of a form with 2 fields has 2 fields, found Text("a")`).
- translate/map.rs:125-145 `Map::field` checks the field id against the type only, not the constructor it now sits in: `field(Identity, Some(Google), "email")` is `Some` after `email` moved down to EmailPass, and walk.rs:1279's `expect` would panic on a stored row. Latent: reached today only through the revert above. Fix: return the later slot and require its constructor to be the image of `c`, or the header.
- mod.rs:454-470 `Pinned::fill` looks only at `AddField`, missing the fill a `revert ... fill` states: pinned after `drop field User.name`, a pinned insert lacking `name` is refused after `revert 0002 fill User.name = "x"` (`it writes no name, which has no default and no fill`), though it translates when the field is re-added with `add field ... fill "x"`. Wrong refusal.

Fix: expand a revert into the inverse operators it applies, once, before translation reads the migration; make `uncrossed` exhaustive over `Resolved`; find a fill by field id from whichever operator brought the field back. Not swept: engine runs of reverted split, merge, nest and unnest; evolve/ itself.

## 2026-11-03: resolved

`fix: translate a revert by the operators it applies`. Cause: translation treated `Resolved::Revert` as one opaque operator: `uncrossed` ended in `_ => false`, `fill` looked only for a logged `AddField`, and `Map::field` matched a field id without its constructor. Class: wildcards and copied rules. Fix: `evolve::Step` carries the operator each inverse step applies; `Pinned::new` expands every revert since the pin once into those operators, resolved by id, each named by the revert's line and carrying its step's fill; `uncrossed` matches every `Resolved` variant under a `#[deny]` that makes clippy refuse a wildcard; `Map::field` answers only where the field sits in the header or the image of the constructor. Tests: crates/adtb-lang/tests/pinned-revert/ (13 operators, both fill cases, a revert of a revert, a two-operator migration; each pair compares the exact reason across the revert and across the written inverse; floors 16 pairs, 20 refusals, 1 refusal-only stand-in), engine answers in bundles.rs, a `Map::field` unit test. 14 mutants killed by name. Two oracles are marked stand-ins in the data: `split` (refusal only) and `addctor` (same answer only). Review found no blockers.

Left: inverse steps that apply no operator (Unadd, Readd, States, Decl, Module, KeyClause, Reorder, Grants) are judged by id through `Map` with no pair in the search; `Pinned::new` replays every revert since the pin on each apply and registration. Both carried to translate-revert-coverage. Found: migration-rename-then-drop.
