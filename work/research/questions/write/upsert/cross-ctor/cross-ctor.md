---
title: An upsert's key may collide with a row of another constructor
pick: any
raised-by: [write/upsert/unless-conflict, write/upsert/put, write/upsert/merge]
options:
  match-else:
    title: The conflicting row is bound as the sum and the else branch matches on it
  retag-else:
    title: The else branch may retag the conflicting row
  ctor-key:
    title: Upserts only on keys whose constructor set is the inserted constructor
  live:
    title: Live with type errors at run time
    excludes: [write/upsert/cross-ctor/match-else, write/upsert/cross-ctor/retag-else, write/upsert/cross-ctor/ctor-key]
---
`insert identities.Google { id: 7, ... } unless conflict on key`
finds identity 7 as an `EmailPass`. The else branch cannot assume a
`Google` row. Postgres refuses an `ON CONFLICT DO UPDATE` that would
move a row to another partition.

## write/upsert/cross-ctor/match-else

```
insert identities.Google { id: 7, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with gid: g }
  | EmailPass e -> fail "identity 7 is email"
```

The conflicting row's type is the key's constructor set, checked
exhaustively.

## write/upsert/cross-ctor/retag-else

```
else match old
  | EmailPass e -> update e set Google { gid: g }
  | Google o    -> update o set Google { o with gid: g }
```

Insert-or-convert in one write.

## write/upsert/cross-ctor/ctor-key

An upsert names a key over the inserted constructor alone, such as
`(gid) in Google`. A collision on a wider key such as the key of
`identities` is an ordinary key violation.

## write/upsert/cross-ctor/live

The else branch is typed as the inserted constructor and fails when the
row is another.
