---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 821
summary: "the spec checks explain B::q(args) at the bundle's pinned version but does not say how argument types translate when the pin differs from the running version"
closed: 2026-10-27
tags:
  - spec
---

# Feature: Argument types of a registered explain at another version

## Report

catalog.md says a registered call is checked at the bundle's pinned version. When the pin differs from the running schema version, nothing states whether the arguments are typed in the pinned version's types or the running version's, or how values translate between them. The checker now takes parameter types in the running schema's types.

## 2026-10-27: done

`applications.md` section 5 states that a call's arguments are values of its parameter types at the pinned version and map forward as a translated write's values do; `catalog.md` section 7.3 types an explained call's arguments at the pinned version. The engine records an explained bundle's parameter types only for a bundle pinned at the current version.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the bundles track.
