---
title: Whether a function may range over any relation with given fields
pick: one
options:
  row-poly:
    title: Row-polymorphic records
    links:
      - {rel: prior-art, to: prior/lang-links}
      - {rel: prior-art, to: prior/lang-machiavelli}
  nominal:
    title: Functions take named relations and constructors only
---
A helper that formats anything with an `email` field, or applies to
`Identity.EmailPass` and `Contact` alike.

## lang/record-poly/row-poly

```
fn domain (r: { email: Text | ρ }) -> Text = split(r.email, "@")[1]
from e in identities.EmailPass select domain(e)
from c in contacts select domain(c)
```

Links row types; Machiavelli kinded record types. Inference stays
decidable.

## lang/record-poly/nominal

```
fn domain (e: Identity.EmailPass) -> Text = ...
```

One function per relation. Simpler errors; reuse through `match`.
