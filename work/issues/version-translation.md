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
