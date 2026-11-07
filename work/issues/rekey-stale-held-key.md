---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 350
summary: "a rekey of r is accepted by the checker and refused at apply, naming no field, when another relation holds Key r in its key"
closed: 2026-11-06
tags:
  - class-none
---

# Bug: a rekey leaves dependent held keys stale

## Report

Seen by the migration-identity-sweep change 2 author (2026-11-06), not reproduced. A rekey of `docs` changes the key of `doc_tags`, a relation keyed by `doc: Key docs`. A `Key doc_tags` held in a third relation's field then names the old key. The carry check does not require that field to be listed, and nobody checked whether anything refuses the migration or rewrites the held key.

Expected: either the rekey rewrites every `Key doc_tags` held anywhere (transitively, as it rewrites `Key docs` it carries), or it refuses naming the field it would leave stale. A stale key stored silently is a broken reference the constraints no longer see.

First step: a script that rekeys `docs` with a third relation holding `Key doc_tags`, asserting the exact rows or the exact refusal.

## 2026-11-06: reproduced by the change 2 review

On main and with change 2: when relation `s` has a key field typed `Key r`, a rekey of `r` that the checker accepts is refused at apply with "a key field has no value in stored keys", naming no field (reviewer's composite-ref.test). Nothing is left stale; the defect is an accepted migration that fails at apply with an unnamed refusal. Fix: either the rekey rewrites the key held inside `s`'s key, or the checker requires it carried and refuses naming the field.

## 2026-11-06 ruled: explicit carry at any depth

The direct case (a `Key r` in `s`'s key, carried) already works on main. The transitive case (a `Key s` field elsewhere, when `s`'s key holds `Key r`) is refused at apply naming no field; evolution.md §4.10 is silent on it. Ruled: the `carry` clause lists every field whose type holds `Key r` directly or through the key of a relation it references, at any depth; the checker requires each and names a missing one; apply rewrites each listed field from the old key of the referenced row to the new. Carry keeps its rule that the author names every field whose stored value the rekey rewrites. Rejected: implicit rewriting (unnamed rewrites), refusing the rekey (loses an expressible migration).

## 2026-11-06: fixed

`fix: require and rewrite carried keys held through another relation's key`. evolution.md §4.10 and the schema-change.md `rekey` row: a rekey of `r` retypes every field whose type holds `Key r` or `Key r.S`, directly or through the key of a relation it references, at any depth, and each must be listed in `carry`; a field of `r`'s own type typed `Key r` directly stays excluded, as the spec already states. Checker: `refers` follows `Key s` into `s`'s key type (`Schema::reached`) and names a missing field. Apply: `Refs::within` follows `Key s` into `layout::key_value_ty(s)`, guarded against key cycles, so the fill rewrites the nested key from the referenced row's old key to its new. Scripts (oplog/rekey-key-held-*, rekey-own-field-held-*): two and three levels and an own-type field held through another key, exact rows forward, after reopen and on revert, and the exact refusals; the transitive script failed on main with "picks: a key field has no value in stored keys". Review: no blockers; the own-type field the checker skipped but apply rewrote, and the spec wording, fixed. `check --changed`: only the ticketed btree timeout; the steps after the suite run by hand, all exit 0.

## 2026-11-06: resolved
