---
title: How a write takes its rows from a query
pick: some
options:
  from-query:
    title: Insert, update and delete take a query as their source
    links:
      - {rel: prior-art, to: prior/write-tutorial-d}
  for-each:
    title: A loop over a query whose body is a single-row write
    links:
      - {rel: prior-art, to: prior/write-edgeql}
---
Bulk writes and per-row writes are one mechanism or two. Updates and
deletes already range over a filtered binder (`update g in
identities.Google where ...`); the question is whether the binder may
range over any query, and how inserts get many rows.

## write/bulk/from-query

```
insert identities.Google from (
  from x in imports where x.kind == "google"
  select { user: x.user, gid: x.sub })

update g in (from g in identities.Google, s in stale where g.gid == s.gid select g)
set Google { g with verified: false }

delete s in (from s in sessions where s.expires < now() select s)
```

The target of `update` and `delete` must be a query whose rows are
rows of one base relation (a key-preserving query), as updatable views
require.

## write/bulk/for-each

```
for x in (from x in imports select x) union (
  insert identities.Google { user: x.user, gid: x.sub })
```

EdgeQL `for ... union (insert ...)`. Per-row writes compose with
`write/surface/expr`.
