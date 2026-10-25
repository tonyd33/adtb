---
title: Whether every stored relation is a sum
pick: one
raised-by: [sum/relation]
options:
  one-ctor-sum:
    title: Every relation is a sum and a plain relation has one constructor
    links:
      - {rel: gains, to: index/ctor-set}
      - {rel: gains, to: evolution/add}
  bare-relation:
    title: Plain relations exist outside sums and a sum groups constructor relations
  two-kinds:
    title: Relations and sums are separate schema kinds with separate keys and FKs
    assessment: {grade: hack, why: "keys, indexes and FK targets get a second mechanism beside ctor-set"}
    links:
      - {rel: rivals, to: index/ctor-set}
---
Each constructor is a relation and a sum their disjoint union. The
question is what a relation that is not obviously a sum (`Session`) is.

## model/one-ctor-sum

`type Session = { ... }` is sugar for a sum with one constructor named
`Session`. Every key, index and FK target is over a constructor set;
for `sessions` the only set is `Session`. Adding a second constructor
is `evolution/add` with no change of kind, and every relation has the
same storage, retag and evolution story.

```
type Session = { identity: Key identities, expires: Time }
-- means
type Session = Session { identity: Key identities, expires: Time }
```

Haskell `data P = P {..}` is a one-constructor sum (Haskell 2010
report sec. 4.2.1, https://www.haskell.org/onlinereport/haskell2010/haskellch4.html).

## model/bare-relation

A relation is the primitive; a sum is a named disjoint union of
relations that the sum declaration owns. A relation outside any sum is
legal and is not a constructor of anything. Constructor-set scopes apply
only inside sums; a bare relation's keys are ordinary.

```
relation sessions : { ... }
type Identity = EmailPass { ... } | Google { ... }
```

Turning `Session` into a sum is a change of kind.

## model/two-kinds

Plain relations are SQL-like tables with keys and FKs; sums are a
separate kind with ctor-set indexes. Each has its own key and FK rules.
