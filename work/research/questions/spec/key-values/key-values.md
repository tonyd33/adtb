---
title: "How a key value is written"
pick: one
options:
  constructor:
    title: "r.key(v) or r.key(f: v, g: w); no literal checks against Key r"
  literal:
    title: "Literals check against Key r by representation"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/key-values/constructor

r.key(v) or r.key(f: v, g: w); no literal checks against Key r. A key names its relation, so it cannot be mistaken for another relation's key.

## spec/key-values/literal

Literals check against Key r by representation. Weakens typed keys.
