---
title: An added constructor is outside every explicit constructor set
pick: any
raised-by: [[evolution/add, surface/ctor-set/list], [evolution/add, surface/ctor-set/named]]
options:
  report:
    title: The add operator lists every set it leaves out and asks
  declare:
    title: The add operator names the sets the constructor joins
  live:
    title: Live with sets staying as written
    excludes: [surface/ctor-set/growth/report, surface/ctor-set/growth/declare]
---
`identities.(EmailPass | Google)` written to mean "all of them" stops
meaning that once `Apple` is added: the unique index and FK targets
quietly exclude `Apple` rows.

## surface/ctor-set/growth/report

Adding `Apple` prints each constructor set over `identities` that does
not contain it; the migration is refused until each is acknowledged or
widened, as the recheck reports non-exhaustive matches.

## surface/ctor-set/growth/declare

```
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
```

The operator states membership; sets are otherwise fixed.

## surface/ctor-set/growth/live

Writing the sum name or `except` is the way to say "all".
