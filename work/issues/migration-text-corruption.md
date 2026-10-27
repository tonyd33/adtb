---
created-at: 2026-10-26
status: open
kind: bug
priority: 799
summary: "an arm clause into a parenthesised match, and splitting a constructor that references itself, both leave corrupt declaration text"
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
