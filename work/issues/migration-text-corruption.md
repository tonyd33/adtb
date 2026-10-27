---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 799
summary: "an arm clause into a parenthesised match, and splitting a constructor that references itself, both leave corrupt declaration text"
closed: 2026-10-26
tags:
  - lang
  - evolution
---

# Bug: Migrations corrupt declaration text

## Report

Found by the campaign manager (`apps/campaign/docs/gaps.md`, "Language: schema evolution"):
- An `arm` clause added to a `match` inside parentheses is placed after the closing parenthesis.
- Splitting a constructor that references itself (`Location.parent`) leaves garbage in the type declaration, and the routing is typed against the old field types.

`campaign::front_end release_1_2_splits_locations` and `release_1_3_adds_quest_states` fail on these.

## 2026-10-26: landed

Landed in `fix(evolution): keep migration text whole through arms, splits and merges, type forward selections within a migration, and add field delete actions`. An `arm` clause now goes after the last arm inside the parentheses. Split and merge drop retyping edits that fall inside a replaced constructor's own text, type the routing at the retyped types, retype products, and rewrite constructor selections in expressions. Every evolve test now checks that the printed schema reparses and rebuilds.

## 2026-10-26: resolved
