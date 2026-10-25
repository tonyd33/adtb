---
title: "Notation for constructor relations and constructor sets"
pick: one
options:
  paren:
    title: "r.C and r.(C | D)"
  brace:
    title: "r.C and r.{C, D}"
---
A constructor relation is named through its relation variable.

## syntax/ctor-set/paren

`identities.Google`, `identities.(Google | Apple)`; a named set `identities.OAuth` declared by `set OAuth of identities = Google | Apple`. The bar mirrors type alternatives and or-patterns.

## syntax/ctor-set/brace

`identities.{Google, Apple}`.
