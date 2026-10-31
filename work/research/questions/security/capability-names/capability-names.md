---
title: What a capability holder may name in the commands its capability allows
pick: one
options:
  any-relation:
    title: A capability command names any relation, and an absent one answers as absent
  grants-only:
    title: Only relations the role holds a grant on
---
`vacuum items` by a role holding `maintain` and no grant on `items`. Reads stay charged.

## security/capability-names/any-relation

An operator runs maintenance on a database without reading its rows. The capability is the trust; it reveals names and nothing of the data.

## security/capability-names/grants-only

Forces a data grant on every operator, widening what operators can read to serve a rule about names.
