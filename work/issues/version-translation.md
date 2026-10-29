---
created-at: 2026-10-27
status: in-progress
branch: version-translation
kind: feature
priority: 822
summary: "a bundle pinned to an earlier version cannot run: reads are not mapped back through the operators' inverses and writes are not mapped forward"
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
