---
title: What an insert returns
pick: one
raised-by: [write/insert/ctor, write/insert/value]
options:
  key:
    title: The keys of the inserted rows
    links:
      - {rel: prior-art, to: prior/write-datomic-tx}
  rows:
    title: The inserted rows, as a relation of the target's type
    links:
      - {rel: prior-art, to: prior/write-edgeql}
  clause:
    title: Nothing unless a returning clause projects it
    links:
      - {rel: prior-art, to: prior/write-pg-constraints}
---
Generated keys must reach the caller and later writes in the same
statement or transaction.

## write/insert/returns/key

```
let id = insert identities.Google { user: u, gid: "g" }
```

Datomic returns resolved `:tempids`.

## write/insert/returns/rows

```
transact
  let g = insert identities.Google { user: u, gid: "g" }
  return key g
```

Inserting into `identities.Google` returns `identities.Google` rows;
inserting constructor values into `identities` returns `identities` rows,
which a `match` consumes. EdgeQL `select (insert ...) { shape }`.

## write/insert/returns/clause

```
insert identities.Google { user: u, gid: "g" } returning id
```

Postgres `RETURNING`, with `old` and `new` qualifiers.
