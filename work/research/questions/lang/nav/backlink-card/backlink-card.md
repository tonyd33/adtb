---
title: A backlink in a scalar position multiplies rows
pick: any
raised-by: [lang/nav/backlinks]
options:
  collection-typed:
    title: A backlink has a collection type and is a type error as a scalar
  flatten:
    title: A backlink in a scalar position flattens into one row per referrer
    links:
      - {rel: prior-art, to: prior/lang-edgeql}
  live:
    title: Live with whichever the lowering does
    excludes: [lang/nav/backlink-card/collection-typed, lang/nav/backlink-card/flatten]
---
`select { email: u.email, provider: u.<identities.user.provider }`: one row per user, or one
per identity?

## lang/nav/backlink-card/collection-typed

`u.<identities.user : Set Identity`; it must be a generator, nested or
aggregated: `count(i for i in u.<identities.user)`.

## lang/nav/backlink-card/flatten

EdgeQL paths over multi links yield a set and the enclosing expression
is evaluated per element, with static cardinality tracking.

## lang/nav/backlink-card/live

No rule fixed.
