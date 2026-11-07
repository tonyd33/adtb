---
created-at: 2026-11-06
status: open
kind: feature
priority: 600
summary: Tag T under merge and nest, held_fills over parametric held values, and nest of a parametric type still refuse
---

# Feature: held-value routing gaps left by change 2

## Request

Left by migration-identity-sweep change 2 (2026-11-06):

- Merge and nest of a type held as `Tag T` refuse. The merged constructor holds its discriminator and the nested one its nesting field, so either tag image loses which source constructor the tag named and the revert has no answer. The spec states no image; ruling 2026-11-06: keep the refusal until the spec states one.
- `held_fills` (add field) skips parametric types held as values.
- The checker refuses nest of a parametric type.

## Proposal

For the first, a spec decision (evolution.md) through the design space before any code. For the other two, route as split now does for parametric held values, with exact rows forward and on revert.

## Open questions
