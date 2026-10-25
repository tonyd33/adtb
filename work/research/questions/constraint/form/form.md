---
title: The primitive form a general constraint is written in
pick: one
raised-by: [constraint/assertion, constraint/inclusion, constraint/cardinality, constraint/fd]
options:
  empty:
    title: A query that must return nothing
    links:
      - {rel: same-as, to: surface/constraint/denial}
      - {rel: prior-art, to: prior/constraint-tutorial-d}
      - {rel: prior-art, to: prior/lang-datalog}
  boolean:
    title: A quantified boolean expression over the database
    links:
      - {rel: prior-art, to: prior/constraint-sql-assertion}
      - {rel: prior-art, to: prior/constraint-alloy}
  implication:
    title: A rule whose body implies its head
    links:
      - {rel: prior-art, to: prior/constraint-logiql}
---
The three are equally expressive: a boolean `b` is the denial of `from
_ where not b`, and `body -> head` is the denial of `body and not head`.
They differ in what a violation reports, what the checker can see
syntactically, and what the dedicated classes are sugar for. Running
example: a user's primary identity belongs to that user.

## constraint/form/empty

```
constraint primary_owned =
  from u in users, i in identities
  where u.primary == key i and i.user != key u
  select { user: key u, identity: key i }
```

Tutorial D `IS_EMPTY(r)`, Datalog denials `:- body.`. The result is the
violation report. One language for queries and constraints, so a
constraint is a registered query rechecked by schema changes.

## constraint/form/boolean

```
constraint primary_owned =
  all u in users: all i in identities where key i == u.primary: i.user == key u
```

SQL `CHECK (NOT EXISTS ...)`, Oracle `ALL (...) SATISFY (...)`, Alloy
`fact`. Needs quantifiers and a boolean sublanguage beside the query
language; a violation reports only `false` unless the engine extracts
witnesses.

## constraint/form/implication

```
constraint primary_owned =
  u in users, i in identities, key i == u.primary -> i.user == key u
```

LogiQL right-arrow rules. Head and body are separate, so a checker
reads off which relations trigger a check (the body) and which are
probed (the head); at-least-one and FKs are this shape.
