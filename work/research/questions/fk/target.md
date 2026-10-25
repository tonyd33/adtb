---
title: What a foreign key may reference
pick: some
raised-by: [sum/relation]
options:
  to-sum:
    title: The sum as a whole
    links:
      - {rel: prior-art, to: prior/gel}
      - {rel: prior-art, to: prior/typedb}
  to-ctor:
    title: One specific constructor
  to-subset:
    title: A subset of constructors
---
Which targets a reference into a relation-level sum can name.

## fk/to-sum

```
type Session = { identity: Key identities, expires: Time }
```

Any constructor satisfies it. Gel links to abstract or union types,
SQL:1999 REFs scoped to a supertable, Oracle substitutable REFs and
TypeDB role players all allow this. Postgres inheritance cannot, for
want of a cross-table unique index.

## fk/to-ctor

```
type Reset = { identity: Key identities.EmailPass, token: Bytes }
```

Only an `EmailPass` identity satisfies it. A reset for a Google identity
is unrepresentable.

## fk/to-subset

```
type OAuthToken = { identity: Key identities.(Google | Facebook) }
```

Any of the named constructors satisfies it. Gel's `A | B` link targets
are arbitrary unions.
