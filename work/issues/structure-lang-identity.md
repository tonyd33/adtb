---
created-at: 2026-11-07
status: open
kind: feature
priority: 16
summary: "about 450 sites key catalog objects by path strings, vector position or name; Schema is open through DerefMut; kernel terms have no validator"
---

# Feature: Catalog identity by id, not spelling or position, in adtb-lang

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-lang.md (local). `Source::ids` keyed by path strings with 7 rewrite arms (evolve/ids.rs `rename_field_ids` rewrites index ids without the constructor); `CatalogIds` by position and name; 114 `ctor_index` sites; `Schema` `DerefMut` to all-pub defs; `u32::MAX` sentinels; kernel terms unvalidated; revert's inverses mirror operators by hand; `Program.checks` aligned with `schema.checks` by position. Subsumes compile-time-identity.

Exit: an `ObjectId` on each definition with `FieldRef`/`CtorRef`; `Schema` closed; a kernel term validator; counts in the report at 0.

## 2026-11-11, design and rulings

Design report: work/briefs/design-lang-identity-r1-report.md (local), on main 34053889. Reproduced live: p1/p1b (renaming a field of one constructor gives a unique and a default over another constructor's same-named field a new id), p2/p3 (renaming a reference field or its constructor gives the implied index a new id), p4 (reverting `drop type` loses value states), p5 (a revert after two renames and a reused name renames the wrong object), p6 (the revert dependency gate compares path strings, so a rename in between hides a dependency). Program: report §4, steps 1 to 10 (5 split into 5a, 5b, 5c).

Rulings (lead):
- Q1: an object's id is carried between operators by a key of resolved ids (a named object by kind, parent id and name; a clause object by relation id, kind, and field and constructor ids). A `Source` keeps declaration text.
- Q2: a typed id per kind in adtb-lang, stored as the same u64, replacing the engine's `Cat*Id` and permission's newtypes; rename keeps an id, drop retires it, none is reused.
- Q3: `CtorRef(TypeId, CtorIdx)` and `FieldRef { def, Header | Ctor, pos }`, minted only by `TypeDef`; `CatalogIds` keyed by them; `Kind::Field` keeps names.
- Q4: `Schema` closes: ids and states are builder inputs, temps are built through the builder, role views are a `Scope`, `RelRef::{Declared, Ahead}`, fields private; `annotate` and `DerefMut` go.
- Q5: the revert dependency gate first becomes a set intersection over `Source::resolved` ids; then each inverse is an operator value by id computed at apply, held by a `Source`-level law search over every operator kind.
- Q6: the kernel validator checks kernel.md §3 typing, sortedness, `Case` arms, `CtorSet` form and binding, §7 normal form as a separate pass; it runs at the exits of check, norm and lowering under debug, the testing feature, the sim and the fuzzer, and once at registration in release; `Terms::add` and `nodes` private.
- Q7: `Program` keys its terms by a typed check or constraint index, read through `program.check(idx)`.
- Compatibility (maintainer, 2026-11-11): no adtb database outside the repository's tests needs to keep opening across these steps. Steps 3 and 5 follow the spec and need no rewrite of stored ids; a fixture holding an old id is regenerated.

## 2026-11-11, step 1 landed

Step 1 landed as ppkzkqul 0a84ccb9, full check 0 (3695 tests). Ten typed ids in adtb-lang (`CatTypeId`, `CatCtorId`, `CatRelId`, `CatFieldId`, `CatSetId`, `CatIndexId`, `CatDefaultId`, `CatComputedId`, `CatFunctionId`, `CatConstraintId`), the engine's and permission's newtypes removed. `CatalogIds` fields private: outside readers 184 to 0, panicking lookups 43 to 0 (41 named `held_id` expects, 2 test unwraps), linear reverse scans 10 to 0, pub fields 21 to 0. Reverse lookups return every holder least first and each site keeps main's tiebreak, since mid-step schemas share ids (mid-migration-ids-shared, for step 3). Plan `IndexId` and `HeapId` kept: they name non-catalog structures. crates/adtb-sim/scripts/api/reopened-file-keeps-catalog-ids.test pins catalog ids against the spec's rule for types, constructors, relations, fields, indexes, defaults, sets, constraints, functions and computed fields; checks, aggregates and stored views are not covered. Review no blocker; final round fixed grant_text lookups back to ids, the least-first test, aggregate and constraint coverage, the reopen script kinds.

## 2026-11-11, step 2 scope

Ruled in step 2: refs only for constructors and fields (Q3). Sets stay keyed by name; uniques and indexes, computed fields, aggregates, constraints, checks and defaults stay keyed by position within their relation or type; view row fields stay by name (an anonymous row has no `TypeDef`). These are for step 6 (closing `Schema`) or a later ruling. The changed `Debug` text of an internal permission error (`MissingIdentity` to `InvalidKernel`) is accepted: the reason stays `Internal`.
Step 2 review residuals, for step 6: refs are not tied to a schema instance (a `CtorRef` minted from one version is accepted by another's `ids.ctor_id`; the two-schema sites mint from the schema they query today); `TypeDef.kind` is `pub` and `Schema` still `DerefMut`; `ImpliedIndex` and `ReferenceField` hold `ctor`/`field` beside a private `field_ref`.

## 2026-11-11, step 2 landed

Step 2 landed as pvsxrmmu b5cf5e11, full check 0 (3702 tests). `CtorRef { def, ctor }` and `FieldRef { def, part, pos }` minted only in schema by `TypeDef` (private `id`); `CatalogIds` keys constructors, fields, implied indexes and defaults by them; `schema::EngineCtors` minted once per build (bool, `Service`, `Role`, `RoleGrant`, `GrantKind.Deployment`, `Violation`), with the `RoleGrant.kind` type check moved into it; `Broken::NoViolation` removed. Header name tests 12 to 1; `CatalogIds` methods by position or name 12 to 0; engine literal-name lookups 11 to 0; constructor-by-name sites 115 to 107, of which 24 are not name resolution (step 3: catalog/moved.rs x10, translate/map.rs; step 9: 5; step 10: 6; test support 2). `catalog::fields` rows keep main's name order (tested); shared-id field holders keep main's order through `Schema::field_holders` (tested), until step 3 makes them single-valued.

## 2026-11-12, step 3 landed

Step 3 landed as mptnnuyk 8e45a75a, full check 0 (3717 tests). `Source::ids` keyed by `ids::ObjectKey` (named objects by kind, parent id and name; clause objects by relation, kind and the field, constructor and set ids they range over); one rename rule; `#n` once in `assign`. Rewrite arms 7 to 0, `BTreeMap<String, u64>` in evolve 6 to 0, path `format!` 19 to 0, holder lists and tiebreak sites 85 to 0; `CatalogIds::set` asserts no shared id. Unlogged objects take provisional ids from a copy of the next-id counter, equal to the reconciled ids; `layout::row` routes by `ctor_fields`. Closes p1, p1b, p2, p3 and mid-migration-ids-shared. Client-visible per catalog.md §1.2: those ids kept; newtype rename keeps its constructor's id; rekey gives the new key index a new id (catalog.md:105); one re-scope rule for split, merge, nest and unnest (a clause listing no replaced constructor keeps its id and text; set names stay); a widened record rename keeps its ids; a merge revert restores set-scoped clauses once; split refuses a shared clause a product does not keep under its name unless replacements are stated. catalog.md `move field` row gained the implied indexes of moved reference fields. Known regressions against main, ignored, for step 3b: clause-element-keys-one-copy (move down, one-constructor set rename). Filed from reviews: full-scope-read-as-unscoped, merge-revert-restates-listed-scope, rename-type-rewrites-other-constructor, rename-sum-refused-by-own-constructor, merge-weakens-joint-unique (all pre-existing on main).

## 2026-11-12, step 3b landed

Step 3b landed as ymrqxrko 4dcf819d, full check 0 (3724 tests). Clause elements key by the field ids they reach across their whole scope (`Part::Reach`, dotted runs resolved through the field's type); every field-replacing operator hands `assign` an old-to-new field map (`Source::replaced`), and kept clauses keep their ids through it; ids an operator creates without replacing anything, and dropped paths, do not count. The one rule: a clause naming no replaced constructor in its scope or any element path keeps its text and id; one that does is re-scoped to exactly the rows it covered. A scope listing every constructor keeps its written scope. A merge refuses while a unique's scope is two or more of exactly the merged constructors. Closes clause-element-keys-one-copy and full-scope-read-as-unscoped. The coverage search (no clause widens or narrows) lands ignored for operators-change-clause-coverage. Filed from the track: split-revert-adds-discriminator, nest-revert-widens-unique, operators-change-clause-coverage (440), split-renamed-carry-drops-set-scope, dotted-elements-skip-value-operators.

## 2026-11-12, step 4 landed

Step 4 landed as usmpsums 6da71341, full check 0 (3734 tests). Revert's dependency gate (`Source::dependants`) intersects, by (space, id), what migration M created (ids and modules present after M and absent before, per evolution.md §5.4 and semantics.md §9.4; ruled: created, not changed) with what each later migration's operators name: resolved operands and clauses, ids reached by texts (canon `Names.ids`), grant field lists (a `C.f` qualifier resolved as a constructor), owners of each named object (`Owner::Object(Space, u64)`, `Owner::Module`), and for a later `revert k` what k named and created. The header module is not an operator and names nothing. The gate runs only for the revert an author wrote, not for one replayed by an inverse. An unresolvable later migration refuses the revert ("the log does not resolve: ..."). Refusal: "`revert N` is refused while migration L depends on it: it names KIND OBJECT". Path vocabulary 21 to 0. Closes p6. Search: 88 histories, 48 refused. Filed revert-gate-counts-reverted-dependants.

## 2026-11-12, step 5a landed

Step 5a landed as ukwmzxut 66357c57, full check 0 (3736 tests); tests only. The law search `every_invertible_operator_and_its_revert_restore_the_source_before_it` (crates/adtb-lang/src/evolve_law_tests.rs) covers all 51 invertible kinds (32 shapes before, 24 kinds never held), 163 cases in the default run (2.7 s), 8 shapes under `deep`. It compares declarations (whitespace outside literals, clauses and match arms as sets, field order kept, unstated `on delete` read as restrict), modules, write and value states, grants by the objects they name, and ids (kept objects keep theirs; re-created objects take the next ids in creation order). `KNOWN` holds 14 rows, each pinned to its exact difference lines and naming tickets that `every_known_row_names_a_ticket` checks exist. p4 (every value state of a dropped type lost) and p5 tests land ignored. Clause boundaries still come from keywords, not the parse (`ast::RelationClause` has no spans). Filed: move-down-revert-moves-header-field-last, structural-operators-refuse-clauses-on-replaced-constructor, merge-revert-with-check-renews-kept-index, revert-drop-relation-loses-grant.

## 2026-11-12, step 5b landed

Step 5b landed as yqkrmrtu ed6e482a, full check 0 (3737 tests). The inverses of rename type, relation, set, constructor and field, add set and drop set are operator values holding catalog ids (`Inverse::Rename`, `AddSet`, `DropSet`, crates/adtb-lang/src/evolve/revert/by_id.rs), resolved to current names at apply and run through the operators and their checks; name-based text inverses of these kinds 7 to 0. p5 passes: reverting 0001 gives `{OAuth: 1024, Federated: 1025}`. 13 law-search cases rename, reuse or drop the object between migration and revert (11 failed on main). A revert whose inverse names an object a later migration dropped is refused with the reason now in evolution.md §5.4. A reused name is refused by the operator's own duplicate check (on main, `drop set R` then a new `add set R` let the revert overwrite the new set). Re-created ids in the search must lie among the ids the revert gave out. Interim, removed in step 5c: `add set`'s inverse names the set by relation id plus name (`Inverse::Replay` re-runs a nested revert against old ids); `renew` maps ids re-created by declaration restores by kind, name and owner. The rename type inverse runs an operator that also renames a constructor of the same spelling (rename-type-rewrites-other-constructor).

## 2026-11-12, step 5c split and rulings

Step 5c's survey on main (p4 fails as recorded; 10 of 14 `KNOWN` rows have an inverse cause, 4 an operator cause; 13 of 16 `Inverse` variants find their object by text or name; 37 name lookups in evolve/revert) proved it larger than one track. Split, in order: 5c-1, value states by typed id from drop type and drop relation, the re-creating step's id map replacing `renew`, grants through it, `add set` by the set's own id, `Replay` threading the map (closes p4 and the three drop rows); 5c-2, restored declarations, clauses, modules, key clauses, reorders, clause and field operators and `Replay` respelled by id; 5c-3, split, merge, nest, unnest, move field, add and remove constructor by id (closes the Merge, Split, Nest and MoveField rows and the RemoveConstructor row).

Rulings: a structural inverse is the inverse operator by id plus the replaced clauses and arms restored as respelled texts in the same step (keeps the operator's checks and row map), not a snapshot restore of each declaration. `StateKey` name tuples in `Source::states` and `value_states` stay until step 6 (close `Schema`); only the `Inverse::States` value holds typed ids now. `remove constructor`'s inverse restores the constructor's set memberships by set id (`joins`), since §5.1 requires the prior schema; the clause coverage half of operators-change-clause-coverage still waits for its ruling.

## 2026-11-12, step 5c-1 landed

Step 5c-1 landed as mzsuuovk 7d4e19cf, full check 0 (3738 tests). Drop type and drop relation emit `Inverse::States` by typed id (`StateOf`), so their revert restores every value state; p4 passes. The revert's id map (`Recreated`, built by `recreate` after each step: full `ObjectKey` match against the objects the reverted operators removed, new ids only, a miss refused later) replaces `renew`; grants map through it; `add set`'s inverse names the set's own id; `Replay` carries the map. Inverses are grouped by runs of operators ending at a source that builds, and the revert declares dropped relations ahead. A states step whose target a later migration dropped is refused (main wrote a stale key); `remove constructor` restores relation states of its own type only. `KNOWN` 14 to 11 (the three drop rows); `renew` 5 to 0; text or name-finding `Inverse` variants 13 to 11. Closed revert-drop-relation-loses-grant. Filed revert-add-set-in-unbuilt-run-unlisted-reason.
