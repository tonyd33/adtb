---
title: How a query reaches related rows
pick: some
options:
  joins:
    title: Explicit generators joined by predicates
    links:
      - {rel: prior-art, to: prior/lang-linq}
  paths:
    title: Path navigation through foreign keys
    links:
      - {rel: prior-art, to: prior/lang-edgeql}
      - {rel: prior-art, to: prior/gel}
  backlinks:
    title: Reverse navigation from a target to the rows referencing it
    needs: [lang/nav/paths]
    links:
      - {rel: prior-art, to: prior/lang-edgeql}
---
These stack: paths are sugar for a join on the FK, and explicit joins
remain for predicates no FK names. Every FK targets a constructor set
(`fk/to-ctor`, `fk/to-subset`, `fk/to-sum`), so a path's type is the
target set's row type.

## lang/nav/joins

```
from i in identities, u in users where i.user == key u select u.email
```

A reference compares with the key of a row.

## lang/nav/paths

```
from i in identities select i.user.email
```

Following `user : Key users` lands on the referenced row. Through an FK
to a sum, the path lands on the sum and its fields are reached by
`match`:

```
from p in payments select match p.method | Card c -> c.last4 | Bank b -> b.iban
```

## lang/nav/backlinks

```
from u in users select { email: u.email, ids: u.<identities.user }
```

EdgeQL `.<user[is Identity]`. A backlink is a collection: many rows may
reference one target.
