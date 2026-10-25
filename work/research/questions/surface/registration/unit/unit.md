---
title: What is registered as a unit
pick: one
raised-by: [evolution/breaks/invisible/registered]
options:
  query:
    title: Each query by itself, identified by its hash
  bundle:
    title: A named client release holding its queries and schema version
---
Retiring, gating and reporting need some grain.

## surface/registration/unit/query

Queries shared by releases are registered once. Nothing says which
client still uses one.

## surface/registration/unit/bundle

```
bundle web-2.3 pins 8 (a91c...)
query login_key (u: Key users) -> Set Text = ...
query list_sessions (u: Key users) -> Set Session = ...
```

A schema change reports failures per bundle; readers-first gates on
active bundles.
