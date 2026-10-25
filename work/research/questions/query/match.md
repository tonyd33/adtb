---
title: How queries consume a sum
pick: some
raised-by: [sum/value, sum/relation]
options:
  construct:
    title: A match construct over constructors in the query language
    links:
      - {rel: prior-art, to: prior/flix}
  plain-sql:
    title: Plain SQL over the union view and CASE on the tag
  narrowing:
    title: A narrowing primitive that filters to one constructor
    links:
      - {rel: prior-art, to: prior/sql-structured-types}
      - {rel: prior-art, to: prior/gel}
      - {rel: same-as, to: query/ctor-relations}
  ctor-relations:
    title: Each constructor exposed as its own relation
    links:
      - {rel: prior-art, to: prior/souffle}
---
How the query language exposes constructors. These stack: a match
construct can be sugar over narrowing or over constructor relations.

## query/construct

```
from i in identities
select match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
```

Binders are typed by constructor; fields of other constructors are not
in scope.

## query/plain-sql

```sql
SELECT CASE kind WHEN 'email_pass' THEN e.email WHEN 'google' THEN g.gid END
FROM identities i
LEFT JOIN email_pass e ON e.identity_id = i.id
LEFT JOIN google g     ON g.identity_id = i.id;
```

Nothing checks that every constructor is handled or that `e.email` is
read only when `kind = 'email_pass'`.

## query/narrowing

```
from i in identities[is Google] select i.gid
```

Gel `[is T]`, SQL:2003 `IS OF (ONLY T)` and `TREAT`. A match is
sugar: one narrowed branch per arm.

## query/ctor-relations

```
from g in identities.Google select g.gid
```

Each constructor is a relation in its own right, joined or unified like
any other, as Soufflé matches `$Google(gid)` in rule bodies.
