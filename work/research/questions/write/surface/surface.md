---
title: Whether writes are expressions in the query language
pick: one
options:
  expr:
    title: Writes are expressions returning the rows they touch
    links:
      - {rel: prior-art, to: prior/write-edgeql}
      - {rel: prior-art, to: prior/gel}
  stmt:
    title: A statement language whose statements contain queries
    links:
      - {rel: prior-art, to: prior/write-tutorial-d}
  txdata:
    title: Queries compute change sets that a transaction applies
    links:
      - {rel: prior-art, to: prior/write-datomic-tx}
      - {rel: prior-art, to: prior/datomic}
---
One language or two: whether an `insert` can appear wherever a query
can, or only at statement level, or not at all as syntax.

## write/surface/expr

```
from g in (insert identities.Google { user: u, gid: x })
select g.id
```

EdgeQL: `insert` and `update` are expressions, nestable and wrapped by
`select`. One grammar, one type checker.

## write/surface/stmt

```
insert identities.Google from (from x in imports select { user: x.u, gid: x.g });
```

Tutorial D: writes are statements, the only things that change the
database, and each takes relational expressions as operands. A query
never has an effect.

## write/surface/txdata

```
transact
  insert identities.Google from (from x in imports select { user: x.u, gid: x.g })
```

As Datomic: `insert ... from` is a change-set value; `transact` applies
it. Reads stay pure; the write language is a type, not syntax.
