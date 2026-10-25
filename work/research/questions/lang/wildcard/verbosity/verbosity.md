---
title: Naming every constructor makes matches over large sums verbose
pick: any
raised-by: [lang/wildcard/scalars-only]
options:
  set-patterns:
    title: A pattern may name a named constructor set
    needs: [surface/ctor-set/named, lang/patterns/or-binder/ctor-set]
  adhoc-wildcard:
    title: Wildcards over sums are allowed in the ad hoc role only
    needs: [surface/adhoc/scoped]
  live:
    title: Live with listing every constructor
    excludes: [lang/wildcard/verbosity/set-patterns, lang/wildcard/verbosity/adhoc-wildcard]
---
A sum with thirty constructors needs thirty arms, and an analyst at the
REPL lists all of them. The ban exists so that adding a constructor
forces a decision about every dependent match; a wildcard makes that
decision silently, before the constructor exists.

## lang/wildcard/verbosity/set-patterns

```
set OAuth of identities = Google | Apple

match i
  | EmailPass e -> e.email
  | OAuth o     -> provider_email(o)
```

`o` has the constructor-set type. Adding `Github` must state whether it
`joins OAuth` (`surface/ctor-set/growth/declare`); joining covers every
such arm at once, not joining makes the recheck report each match. The
decision is made once, where the constructor is introduced.

## lang/wildcard/verbosity/adhoc-wildcard

Ad hoc queries are never rechecked and carry no guarantee, so `_` is
allowed over sums there.

## lang/wildcard/verbosity/live

Every arm written out.
