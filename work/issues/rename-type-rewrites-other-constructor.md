---
created-at: 2026-11-12
status: open
kind: bug
priority: 350
summary: "renaming a type rewrites another type's constructor of the same name, giving it and its fields new ids"
tags:
  - class-identity-by-spelling
---

# Renaming a type renames another type's constructor

## Report

`type X = P | Q` and `type U { id: Key us } = X { a: Text } | Z`; `rename type X to X2` also renames U's constructor `X` to `X2`: its id goes from 1028 to 1031 and `U.X.a` from 1025 to 1026, and U's stored tag changes. Cause: the rewrite in evolve/cx.rs (around 547-576) replaces the word in every declaration, not only the type's own. Pre-existing on main.

Direction: the rename rewrites references resolved to the type by id, not by spelling. Test: the scenario keeps U's constructor named `X` with its ids.

Defect class: Catalog rows and identity by position or spelling.
