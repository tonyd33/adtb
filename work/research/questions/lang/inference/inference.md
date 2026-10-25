---
title: How much of a query's typing is written
pick: one
raised-by: [query/compile-time]
options:
  inferred:
    title: Everything inferred from the schema; annotations optional
    links:
      - {rel: prior-art, to: prior/lang-machiavelli}
      - {rel: prior-art, to: prior/lang-links}
  signatures:
    title: Function and parameter signatures written; bodies inferred
  annotated:
    title: Every binder and result annotated
---
Exhaustiveness is a type check (`query/compile-time`), so every query is
typed against the schema before it runs. Generators over schema
relations already fix most binder types.

## lang/inference/inferred

```
fn logins (u) = from i in identities where i.user == u select i   -- u : Key users inferred
```

Machiavelli and Links infer types of whole queries, record operations
included.

## lang/inference/signatures

```
fn logins (u: Key users) -> Set Identity = from i in identities where i.user == u select i
```

Rust and Haskell practice: signatures at definitions, inference inside.
Registered queries are checked against a stated contract, so a schema
change reports a signature broken rather than an inferred type changed.

## lang/inference/annotated

```
from (i: Identity) in identities select (i: Identity)
```

No inference; noisy, with no precedent in a query language found.
