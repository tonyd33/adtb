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

## 2026-10-29: lead rulings on the carry choices

The track (032da77a) raised six readings; lead rulings, all observable through ad hoc refusal:

1. A constructor joining a named set a grant names: an unlisted `read`/`update` grant on the set covers every field the constructor has when it joins, as the whole-relation rule does. Covering the constructor with no fields would refuse a whole-binder query over the set after the join, contrary to 3.2's "covers a constructor that joins the set". A listed grant on the set covers the listed fields the joining constructor has. security.md 3.3 should state this; send to spec-revision with the next security pass.
2. Split and merge products are not constructors a whole-relation grant gains; 3.6 governs, and a computed field of a product is not covered even under an unlisted whole-relation grant.
3. A merge discriminator is covered by a grant that covers every source constructor; it reveals only which source a row came from, which that grant already shows.
4. A split product reached by several arms covers a field only if every arm carries it whole from a covered field.
5. A nest of a partly granted field set is refused unless restated, as a merge is.
6. Granting the same unlisted privilege twice with different frozen field sets keeps two entries; an unlisted revoke removes both.

Gap for a future dump format: state alone cannot print an unlisted whole-relation grant whose relation later gained constructors; the log can.

Ruling 3 holds for listed grants too: any `read` on every source already shows a row's source constructor by matching, so a listed grant covering every source also covers the discriminator. Implemented in 76bdfd3a.

## 2026-10-29: first review blocks 76bdfd3a

Fresh review (work/briefs/review-grant-carry.md, local) found four blocking defects, each with a witness. Lead rulings:

- B1: a revert leaves stale grants when a later operator's undo recreates objects under new ids (`Inverse::Grants` keeps only the ids before). Record the ids after the operator too and translate through them.
- B2: `move field` up to the header with a `fill` gives a grant computed values. Ruling 7: the moved header field is covered by a grant only if, on every constructor the grant covers, the fill carries a covered field whole; otherwise it is not covered, with no refusal, as a split's computed field. Live bundles that lose it are the recheck's concern (next slice).
- B3: a nest whose sources are all partly granted the same way applies and leaves the grant with no fields. Ruling 5 holds: refused unless restated.
- B4: after a merge an `update` grant covering the discriminator can retag between sources. Ruling 8: an `update` grant covers the discriminator only when the role held `retag` between every ordered pair of the sources; ruling 3 is about `read`.
- Non-blocking, to fix in the track: `spread` adds field ids of constructors the grant does not cover; privilege kinds compared as strings in `Unstated`; the merge refusal names neither privilege nor relation; four tests check one element instead of the whole list; schema-change.md's revert bullet and move-up rule; `unstated` lives on `Source` instead of the migration state.
- Left: a listed revoke after a carry must name the carried field set exactly, and a revoke matching nothing reports nothing (spec-evolve-gaps covers revoke of a privilege never granted). A listed set grant naming a field some constructor lacks is accepted, against security.md 3.3; that predates this track.

## 2026-10-29: second review blocks eaae63ba

B1 to B4 are fixed in principle (work/briefs/review-grant-carry2.md, local). Branch check on eaae63ba passed (root 2760, campaign 47). Two holes of B4's kind, present since 032da77a, with lead rulings:

- B5: a nest gives `update` on the nesting field to a role without retag between the sources. Ruling 9: the nesting field is covered for `update` only when the role held `retag` between every ordered pair of sources, as ruling 8; no refusal.
- B6: a split or unnest carries each side of a retag separately, so a retag between products of one source appears, which before was an `update` of the routed fields. Ruling 10: such a pair is carried only when the role held `update` on that source covering every field the routing matches; otherwise the pair is dropped, with no refusal, since the role never held it.
- Non-blocking to fix: duplicate refusal lines for one nest; `retags` computed for every kind; schema-change.md's revert bullet says the removing step names new ids.
- Left: a guarded fill arm counts as not carrying while a split's routing ignores guards (fill side only stricter); a no-op `retag X -> X` is accepted and satisfies the merge's retag condition (predates); security.md 3.6 could state that a nest or merge of sources granted by separate grants is refused unless restated.

## 2026-10-29: third review blocks 5e9fad90

B5 and B6 hold for every listed shape (work/briefs/review-grant-carry3.md, local). Branch check on 5e9fad90 passed (root 2765, campaign 47). Lead rulings:

- B7: a split arm's guard that reads a header field is not counted among the routed fields, so ruling 10 keeps a retag between products routed by that header field. Count every field a guard can read, header fields included.
- B8: reading a split or unnest product shows which product a row is, so the routed value. Ruling 11: kept. security.md 3.6 gives each product the old constructor's grants, and a migration is written by a role holding `schema`, which could grant the routed field outright. The product tag is the schema author's disclosure, not a carry leak. Rulings 9 and 10 differ because they would give a write the role did not hold. spec-revision: 3.6 should say a product's constructor reveals its routing.
- B9: dividing a whole-relation or set retag into constructor lists breaks a revoke by the original text, and stops covering constructors added later. Ruling 12: a divided retag keeps its targets as written and records the constructor pairs it excludes; a revoke by the written text removes it, and a constructor added later is covered as before, outside the excluded pairs.
- Non-blocking to fix: `divide_retags` returns early on a schema error and keeps every pair; its unreachable `else` branch; the `td_fields` helper; schema-change.md's ruling 10 bullet and rulings 11 and 12.

## 2026-10-29: fourth review blocks a03cc1ce

B7 and B9 hold; B8 behaves as ruled (work/briefs/review-grant-carry4.md, local). Branch check on a03cc1ce passed (root 2770, campaign 47).

- B10: a later operator drops a retag's excluded pairs instead of carrying them (`carry_plan` maps no held pair; pruning drops pairs naming a gone constructor), so a second split, a merge or a nest gives back a retag ruling 10 removed. Ruling 13: excluded pairs follow their constructors through every later operator, each replaced constructor becoming every product it reaches. A merge or nest whose sources differ in the excluded pairs a retag grant holds is refused unless the migration restates that retag, as ruling 5.
- Non-blocking to fix: a test that a merge of two mutually excluded products withholds `update` on the discriminator; schema-change.md states rulings 13 and that a guarded split needs `update` on every field of the source to keep a retag whole.
- Left: a regrant after a division holds both the whole retag and the excluded one.
