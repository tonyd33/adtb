---
title: "Module syntax"
pick: one
options:
  header:
    title: "module name at the head of a file"
  block:
    title: "module name with a braced body"
---
Modules namespace schema and bundles (`surface/namespace/modules`).

## syntax/module/header

`module auth` then declarations, one module per file. A bundle is `bundle web pins 8 (a91c...)` in its own file importing modules.

## syntax/module/block

`module auth { ... }`, several per file.
