---
title: Grouping several writes into one atomic statement
pick: any
options:
  multi:
    title: A multiple-assignment block applied as one write
    links:
      - {rel: variant-of, to: constraint/timing/multi-assign}
      - {rel: prior-art, to: prior/write-tutorial-d}
  nested:
    title: An insert as the value of a reference field
    links:
      - {rel: rivals, to: model/deferred-check, for: inserting reference cycles}
      - {rel: prior-art, to: prior/write-edgeql}
---
Creating a `User` with its first `Identity` writes two relations with a
reference between them. A transaction always groups statements; this
asks whether one statement can, which decides whether a statement-level
check can hold that every user has an identity.

## write/unit/multi

```
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key u
```

As RM Prescription 21: the block's writes are one multiple assignment;
constraints hold at its end.

## write/unit/nested

```
insert identities.Google {
  user: key (insert users { name: n }),
  gid: g
}
```

EdgeQL nested inserts. Covers trees, not cycles.
