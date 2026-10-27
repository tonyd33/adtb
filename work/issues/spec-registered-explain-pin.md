---
created-at: 2026-10-26
status: open
kind: feature
priority: 821
summary: "the spec checks explain B::q(args) at the bundle's pinned version but does not say how argument types translate when the pin differs from the running version"
tags:
  - spec
---

# Feature: Argument types of a registered explain at another version

## Report

catalog.md says a registered call is checked at the bundle's pinned version. When the pin differs from the running schema version, nothing states whether the arguments are typed in the pinned version's types or the running version's, or how values translate between them. The checker now takes parameter types in the running schema's types.
