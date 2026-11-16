---
created-at: 2026-11-14
status: open
kind: feature
priority: 30
summary: "Schema::declared panics on a mid-migration schema, held only by a runtime check; a committed schema type carrying declared would make the precondition a type"
tags:
  - structure
  - class-failure-flattened
---

# Feature: a committed schema type that alone carries `declared`

## Report

`Schema::declared` (crates/adtb-lang/src/schema/mod.rs) reads a key's relation and panics on a schema with relations a later operator adds; since the ahead-declared track it panics whenever `ahead` is non-empty, whatever key it is given. Three of its callers were judged committed-only and proved reachable mid-migration (step 6b rounds 2 and 3, fill-print-of-ahead-key-panics). The precondition is held by tracing each caller's schema to where it is made, and by that panic.

Direction: a `Committed` schema type that only a schema with no ahead relations can give, carrying `declared`, held by every caller's schema source without an `expect` at each. Sources measured: engine `Catalog.schema` (catalog/mod.rs:170), the write path's `Arc<Schema>` fields (write/mod.rs:470, 601, 658), api `Arc<Schema>` fields (api/mod.rs:349, 762, 782; api/registered/mod.rs:174, 187, 301, 493; api/server/call.rs:118; api/text.rs:57; api/maintenance.rs:53, 106), the reference database's schema, `Signature.schema` (signature.rs:332), translate `Map.from`, codegen, adtb-file functions taking `&Schema`, sessions made by `with_temps`; producers `build`, `build_with`/`relabelled`, `Source::schema`, `with_temps`. 20+ fields across five crates: more than one track. First step: `Catalog.schema: Committed` with `Deref<Target = Schema>`.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-16, count and split

Counted on 9e4804be (receipt-committed-schema-r1): `Schema::declared` panics only on a `RelRef::Ahead` key (the whole-schema check was withdrawn in ahead-declared round 4). 16 callers outside tests: 11 with a committed source; 5 in translate over `Map.from`, which a reverted fill builds with ahead relations (they meet only declared keys today; no history reproduced reaching an ahead key). Temps sessions copy their base's `ahead` and add none. Split, landed one at a time: 4 (translate's 5 sites to `relation_of`, an ahead key a translation refusal), 1 (`Committed` in adtb-lang: wraps a private `Schema`, `Deref` only; `build` returns it; `Committed::new -> Option` checks `ahead` empty; `Catalog.schema` and `Checked.schema`), 2 (write path and api fields, `with_temps` on `Committed`, `check_adhoc`/`Adhoc::schema` over `&Committed`), 3 (bundles, codegen, reference). Rulings: `Committed::declared` on an `Ahead` id panics naming the rule (an id from another schema), as `relation` does for a foreign id; the last part removes `Schema::declared`.

## 2026-11-16, part 4 widened

The review of part 4 (rrrmqont) found a valid history refused: a V1 insert pinned across `add field User.l: Option (Key later) fill None` then `add relation later` is refused at open ("no `later` in scope", pinned-forward-fill-later-relation-refused), because translation's `replayed` clones the logged source without `expecting`; part 4's own refusal would refuse it too, against evolution.md §7.2 and §7.4. Rulings: an ahead relation resolves by identity, the position of its adding operator in its migration (for a revert, of the inverse step that adds it back), not its name and not a pre-assigned catalog id; the position is passed explicitly from the enumeration that yields the operator. Parts: 4a-1 forward link (pending-ness and `late_sels` by identity), 4a-2 revert link (`inverse_ahead` goes; closes ahead-relations-keyed-by-name, readers that resolve operator or source text count as name resolution), 4b translate (`replayed` calls `expecting`, an ahead key resolves through its operator's `Resolved::AddRelation` to `Map.to`, `Loss::Ahead` goes; closes pinned-forward-fill-later-relation-refused). Parts 1 to 3 follow.

## 2026-11-16, part 4a-1 landed

Landed as ystwnzvs 7e7cf707 (2 rounds), full check 0 (3834 tests). `evolve::Listed` carries an operator with its position from the enumeration that yields it; `Source.ahead` is a list of `Expected` linked to their adding operator (`Adder::Operator(k)`, or `Adder::Inverse` until 4a-2); pending-ness and `late_sels` go by that link, and a pending entry whose name is currently declared makes no ahead entry (name resolution of text). The review found that a shadow ahead entry refused add, drop, add of one relation; fixed with scripts oplog/relation-added-dropped-added-again.test and oplog/relation-renamed-away-then-added-again.test. Readers comparing an ahead relation's name 5 to 3, all text resolution. Filed: add-drop-refused-while-other-add-pending. Next: 4a-2.

## 2026-11-16, part 4a-2 landed

Landed as wuztvzvl df7fafa3 (2 rounds), full check 0 (3837 tests). `Inverse::RestoreRelation` carries its `typed`, read from the relation's declaration where the drop applied (the old lookup in the source before the whole migration missed after a rename); `linked` and `undo_all` enumerate the same steps vector, so the restore position comes from the walk that applies it. Resolved: ahead-relations-keyed-by-name. Next: 4b (rebase rrrmqont 4cad9496).

## 2026-11-16, part 4b landed

Landed as pkrpqpxz e006c7e8 (2 rounds), full check 0 (3839 tests); rrrmqont folded in. `Schema.ahead` entries carry their `Adder`; translate's 5 `declared` sites resolve through it (`Later::At`, `Lacked` refused as `Loss::Later`, `Unlinked` an invariant held lazily, since a reverted fill's schema can list a restored relation it holds no key of). The review found a valid history (a function parameter carrying the key, the relation dropped by a later migration) reaching the first version's panic; it is a refusal. `Schema::declared` callers outside tests 16 to 11, all committed. Resolved: pinned-forward-fill-later-relation-refused. Next: part 1, `Committed`.

## 2026-11-16, part 1 landed

Landed as kqrtyvnw e50c05c5 (2 rounds), full check 0 (3840 tests). `Committed` (adtb-lang schema/committed.rs) wraps a private `Schema` with `Deref` only; `Committed::new` accepts only a schema no migration is being applied to (`ahead` and `late_sels` empty); `build` returns it. `Catalog.schema`, `Checked.schema`, `fold` and `engine_schema()` hold it; a `None` from a producer is `Broken::Unsettled` (every producer settles its source first). `Committed::declared` exists and is not called yet. Next: part 2 (callers onto `Committed::declared`, `check_adhoc`/`Adhoc::schema` over `&Committed`, the two registered `Arc<Schema>` clones), then part 3.

## 2026-11-16, cause review: ahead links eager and total

The cause review found two patches in part 4: `schema/build.rs:78-79` still decides pending-ness by whether the name is declared (`Pending if named`, `UnlessDeclared if named`), and `translate/map.rs:257` panics on `Later::Unlinked` on a premise ("a reverted fill holds no key of a relation its revert restores later", held by the `drop relation` refusal) that part 4b's final round refuted for the forward half (a function parameter carries the key past it). Ruling: a part 2b after part 2, before part 3: `declare_ahead` decides by the `Adder` link alone (the named arms go, add-drop-refused-while-other-add-pending fixed by it); `Unlinked` becomes a `Loss` refused like `Lacked`, checked when the fill map is built for a key the fill holds; a searching test over add, drop, rename and revert sequences of two relations, each translated and replayed. Resolves add-drop-refused-while-other-add-pending, translate-reverted-fill-ahead-key and pinned-reverted-fill-key-internal if it covers them; the track checks each.

## 2026-11-16, part 2 review

The review of part 2 found a late selection widened for script writes (script-late-selection-widened, not live: the engine runs no script). Ruled into part 2b: a script that writes or matches by constructor through a field whose late selection has not resolved is refused. Part 3 also takes `Ids::of`/`signature_ids` over `&Committed` and deletes `Schema::declared`.
