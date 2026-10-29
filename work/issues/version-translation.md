---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 822
summary: "a bundle pinned to an earlier version cannot run: reads are not mapped back through the operators' inverses and writes are not mapped forward"
closed: 2026-10-29
depends-on:
  - bundles
tags:
  - engine
---

# Feature: serve a pinned version by translation

## Report

`evolution.md` section 7: a bundle pinned to version `v` reads the current data mapped back to `v` through the inverses of the operators since `v`, writes through the operators forward, and encodes results at `v`. Section 7.2 decides translation at apply, 7.3 states expired bundles, and 7.4 the fields a translated write lacks. The `bundles` track pins and checks bundles but serves only bundles pinned at the current version.

Ruling for `spec-registered-explain-pin`: a registered call's parameters are typed at the bundle's pinned version, and its argument values map forward to the current version as written values do; its results are encoded at the pinned version.

The schema operator law of `oracle.md` section 5.7 for queries pinned to a version are untested until this lands; add them to `schema_operators_keep_their_laws` in `crates/adtb-testkit/tests/differential.rs`.

## 2026-10-27: bundles landed

A bundle pinned before the current version registers and is checked at its pin, and its calls are refused `Untranslatable`. A migration is not yet refused for a live bundle whose query has no translation (section 7.2).

## 2026-10-28: dispatched

A fresh writer owns this in workspace version-translation on main 1790a065, alone. It writes a plan first (work/briefs/plan-version-translation.md, local), then slices in order: the translation pass and serving calls for rename, add field with section 7.4 fills, unread drop field, add constructor, state changes and no-change operators; section 7.2 refusal at apply; split, merge, nest, unnest, move field, rekey and sets; definitions at the pin. A slice not reached stays refused `Untranslatable`. Gates: targeted adtb, adtb-lang, adtb-sim and adtb-testkit tests, clippy, fmt, house-check, size-check; the lead runs the full check. Local brief: work/briefs/brief-version-translation.md.

## 2026-10-28: writer froze the source

Frozen e94a474c677cd2d73fb74daad073b53f8f8c0d10 on main 1790a065 (workspace version-translation). New `adtb_lang::translate` over a callable's normal form at its pin maps relations, types, constructors and fields by catalog id and rebuilds the query over the current schema; section 7.4 fills come from the replaced row, then the current default, then the add-field fill. Registration translates each bundle pinned earlier and refuses `Untranslatable { query, detail }`; calls run through the translation with arguments recoded forward and results back to the pin (api/pinned.rs); before each migration `registered::recheck` translates every live bundle and refuses `SchemaRefused { operator, detail }` naming bundle and query.
Done: rename, add field with the three fills, unread drop field, add constructor, enable/disable/deprecate, no-change operators, refusal at apply, sets and definitions at the pin through normalisation. Still refused, naming the operator: split, merge, nest, unnest, move field, alter field and rekey when the query reaches them; arguments whose type gained or lost a field; `at` blocks and file reads in pinned bundles; fills from a revert's `Fill` clause. Set union, comparison or ordering over a type that gained a field is refused rather than translated.
Writer gates: clippy, fmt, house-check, size-check; adtb-lang and adtb 1060 passed; adtb-sim 917 passed, 5 skipped; adtb-testkit 47 passed, 2 skipped. The pinned-query law is in `schema_operators_keep_their_laws`. Running: the lead's branch check and a fresh review.

## 2026-10-28: review blocked; fixes dispatched

The lead's branch check of e94a474c failed only adtb-manual errors::line_404 (the manual still described call-time `Untranslatable`); the writer fixed the manual at 1058efd3. The independent review (work/briefs/review-version-translation.md, local) blocks on five findings: B1 count, sum, avg, aggregates, top, take and only over a type that gained a field are not refused, so a count can differ from the pin's; B2 `reached` does not follow field types, so a split of a type held inside a whole row applies and returns the new constructor's tag; B3 an argument of a removed constructor is read as the next one; B4 a whole value written by an update ignores section 7.4 rule 1; B5 an expired bundle registering again is refused `Untranslatable`, not `BundleExpired`. Also: an empty operator in a refusal, the pinned law vacuous for the refused operators and blind to B1 and B4, and missing tests for disable, remove, upsert and reopen after a refused migration.
Lead rulings: registration of a bundle with an untranslatable query is refused `Untranslatable { query, detail }` (applications.md section 4.1 rule 3 names no error; the spec wording is a follow-up); translating an un-inlined call to a function redefined since the pin stays refused, naming the operator, as a follow-up.
A fresh fix writer continues in the same workspace from 1058efd3. Local brief: work/briefs/brief-version-translation-fixes.md.

## 2026-10-29: second review blocking, fixes dispatched

The fixes for B1-B5 were rebased onto main bf2c4ff2 without conflicts. A second fresh review (work/briefs/review-version-translation2.md, local) found B2 and B3 fixed and four blocking findings: N1, the carries-whole check keys its seen set by type alone, so a later occurrence with other constructors is skipped; N2, `distinct` exempts sets by element type, but a field-read set of a relation's element type is not distinct by key; N3, `for`/`case`/`let!`-bound whole values skip the rebuild, so section 7.4 rule 2 defaults never run; N4, a superseded bundle registered again with no translation is refused `Untranslatable`, not `BundleExpired` (catalog.md errors table). Lead rulings: N2 exempts by where the set comes from (rows read from a relation), N3 decides by write site (an update's replaced row takes rule 1, every other whole value rule 2). A fresh writer fixes N1-N4 plus `open_translated` hiding engine refusals, the duplicated served state, `blame_ctor` preference and exact test details. The rest is in version-translation-follow-ups. Local brief: work/briefs/brief-version-translation-fixes2.md.

## 2026-10-29: landed

Landed as df87fbce after three fresh reviews; the third was clear and the main check passed on it. A bundle pinned at an older schema version is served by translating each of its queries and transactions to the current version (adtb-lang `translate`); a migration a live bundle cannot cross is refused naming the operator, and registering a bundle with a query that has no translation is refused `Untranslatable`, or `BundleExpired` when the bundle is expired or superseded. Section 7.4 missing-field rules apply by write site. The pinned law in crates/adtb-testkit/tests/differential.rs checks every translated query against the plain query. Open items, including the retired-bundle error order and the errors-table wording, are in version-translation-follow-ups.

## 2026-10-29: resolved
