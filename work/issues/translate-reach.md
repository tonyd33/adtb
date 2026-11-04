---
created-at: 2026-11-03
status: resolved
closed: 2026-11-03
kind: bug
priority: 120
summary: "translation misses a key's field types and the relations a query writes, and carries the key by role, so a pinned insert is silently skipped after a rekey and a pinned key panics after a split"
---

# Translation reach misses keys and written relations

## Report

Found by the translate audit (identity by name or position, after three older wrong answers in translate this week). Reproduced through the engine.

- crates/adtb-lang/src/translate/mod.rs:353-361 `uncrossed`: for a relation it adds the element type but never its key's field types. Relation `p` keyed `(t: Tier, n)`, a pinned query returning `qs.o : Key p`, then a split of `Tier.Paid` and a Monthly row inserted: the pin answers `p.key(n: "a", t: Free)`; after, translation is accepted and the call panics at crates/adtb/src/api/pinned.rs:42 (`a carried value's constructor has a counterpart at both versions`). Fix: `reached` follows `Key { rel }` into the key's field types.
- mod.rs:380-383, the Rekey arm, and walk.rs:410 `Conflict::Key`: the relations checked come only from `Key` types, so a relation the query writes is missed, and "the key" is carried by role, not by which key. Prices keyed `(sku, n)`, then rekeyed to `sku`; a pinned `insert prices {sku: "a", n} unless conflict on key else skip`: at the pin `put(2)` inserts `(a, 2)`; after, `put(2)` answers Ok and skips, rows stay `["1"]`. evolution.md 7.1 says the old key stays enforced and 7.2 that a translated write violating a later constraint fails as a violation. Fix: collect relations from Rel, Insert, Upsert, Replace, Delete, KeyOf and Fresh terms; give the key an identity and map `Conflict::Key` as `unique` maps a unique, refusing when the pinned key is gone.

Defect class: identity by role or reach instead of catalog id. Receipt with probes: work/briefs/receipt-translate-audit-r1.md (local).

## 2026-11-03: resolved

`fix: translate a pinned key and a written relation by identity`. `Schema::reached` follows `Key r` into the key fields' types, at the relation's type arguments, once per relation, so `uncrossed`, readers-first (`blocks_value`) and `stored_in` see them: the split of a sum in a key field is refused under a pinned key, and enabling a constructor a read key can hold is refused while a bundle reads that key, where it was admitted. walk.rs `same_key` maps a pinned `Conflict::Key` upsert and a `Replace` by the set of key field catalog ids, refusing a rekey that changes it ("it writes by the key of `R`, which the current version replaces"). A relation a query writes needs no collection of its own: every such term carries its relation in its type. Search: crates/adtb-lang/tests/pinned-keys/ (fixtures parsed by crates/adtb-testkit/src/sections.rs), 72 pairs in translate/tests.rs, 42 engine answers in bundles.rs, a readers test. Eight mutants fail by name. Full check 3654 passed. Class: catalog rows and identity by position or spelling.

Lead ruling: a widening rekey is refused under a pinned write on the key, though evolution.md 7.1 keeps the old key enforced and a pinned write could be checked against it, because translation has no conflict target that names the held key. Interim, like pinned-print-parse-translation.

Left: the engine search takes 1.9 s alone and 3.25 s under the full suite; mutual keys (a's key holds `Key b`, b's holds `Key a`) terminate by reading but have no test. Found: migration-key-rename-and-split-refused.
