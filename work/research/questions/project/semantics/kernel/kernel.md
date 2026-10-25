---
title: What the semantics is given on
pick: one
raised-by: [project/semantics/denotational, project/semantics/mechanised]
options:
  core:
    title: A small kernel calculus; the surface is defined by desugaring to it
    links:
      - {rel: prior-art, to: prior/lang-nrc}
  surface:
    title: Every surface construct directly
---
Settled rulings already call most of the surface sugar: paths and
backlinks over generators on typed keys (`lang/nav/paths`), `match`
over constructor relations (`query/construct`), higher-order
abstraction normalised away (`lang/abstraction/higher-order`).

## project/semantics/kernel/core

Kernel: comprehensions over sets, constructor relations and their
disjoint union, `fix` with stratified negation, nested results, and
change sets. The spec gives the kernel's meaning and each surface form
as a rewrite into it. GHC Core and the WebAssembly abstract syntax
play this role (unverified as stated scope).

```
-- surface
from u in users select { id: u.id, ids: u.<identities.user }
-- kernel
from u in users select { id: u.id, ids: from i in identities where i.user == key u select i }
```

## project/semantics/kernel/surface

One semantic equation per construct users write. No desugaring
step to trust, more equations to keep consistent.
