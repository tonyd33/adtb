---
title: How constraints beyond keys and references are declared
pick: some
options:
  check:
    title: Per-constructor checks over the row's own fields
  denial:
    title: Named queries that must return nothing
    links:
      - {rel: prior-art, to: prior/lang-datalog}
---
Keys, FKs and participation have their own syntax. Other invariants
need a form.

## surface/constraint/check

```
check email_has_at on identities.EmailPass (contains(email, "@"))
```

Row-local, checked on each write to that constructor's heap.
Postgres `CHECK`.

## surface/constraint/denial

```
constraint one_primary =
  from u in users, i in identities.Google, j in identities.Google
  where i.owner == key u and j.owner == key u and i.id != j.id
        and i.primary and j.primary
  select u
```

Any query; a write that makes it non-empty is refused. SQL-92
`CREATE ASSERTION`, which major engines never implemented (unverified),
and Datalog integrity constraints. Incremental maintenance decides the
cost. Being a registered query, it is rechecked by every schema change.
