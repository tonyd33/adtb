---
title: How an insert names the constructor of a new row
pick: some
raised-by: [sum/relation]
options:
  ctor:
    title: The target is a constructor relation
    links:
      - {rel: gains, to: query/ctor-relations}
  value:
    title: The target is the sum and each row is a constructor value
---
A row of a relation-level sum has exactly one constructor, so an insert
must fix it. `insert identities { ... }` with no constructor is a type
error under either option.

## write/insert/ctor

```
insert identities.Google { user: u, gid: "g-123" }
```

Symmetric with `from g in identities.Google`. Fields are those of
`Google` plus the sum's key; the key may be generated.

## write/insert/value

```
insert identities from (
  from x in imports
  select match x.kind
    | Email e -> EmailPass { user: x.u, email: e.addr, hash: e.h }
    | Oauth o -> Google { user: x.u, gid: o.sub }
)
```

One statement inserts rows of several constructors; the constructor is
data, computed per row.
