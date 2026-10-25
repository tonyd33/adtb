---
title: How a write inserts or updates on a key
pick: some
raised-by: [index/ctor-set]
options:
  unless-conflict:
    title: An insert with an else branch run when a named key collides
    links:
      - {rel: prior-art, to: prior/write-edgeql}
  put:
    title: Insert only if no row with the key exists, returning the row either way
    links:
      - {rel: prior-art, to: prior/write-typeql}
      - {rel: prior-art, to: prior/write-datomic-tx}
  merge:
    title: A source joined to a target with matched and unmatched arms
    assessment: {grade: costly, why: "a target matched twice errors, and concurrent inserts are not arbitrated"}
    links:
      - {rel: prior-art, to: prior/write-sql-merge}
---
Every key is an index over a constructor set, so an upsert names that
index, the key of `identities` or `unique (gid) in Google`.

## write/upsert/unless-conflict

```
insert identities.Google { user: u, gid: g }
unless conflict on (gid) in Google
else update old set Google { old with user: u }
```

EdgeQL `unless conflict on .k else (...)`, against an `exclusive`
constraint; `old` binds the existing row.

## write/upsert/put

```
put identities.Google { user: u, gid: g } on (gid)
```

TypeQL `put` inserts the pattern unless it matches; Datomic tempids
unify through `:db.unique/identity`. No else branch.

## write/upsert/merge

```
merge s in imports into g in identities.Google on s.gid == g.gid
  | matched     -> update { user: s.u }
  | not matched -> insert Google { user: s.u, gid: s.gid }
```

SQL MERGE. Postgres points concurrent upserts at `INSERT ON CONFLICT`
instead.
