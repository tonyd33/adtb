---
title: What a policy predicate may read
pick: one
raised-by: [security/policy/predicate]
options:
  row-only:
    title: Fields of the row and the caller
  relations:
    title: Any relation, recursion included
    links:
      - {rel: prior-art, to: prior/security-zanzibar}
      - {rel: prior-art, to: prior/security-sqlserver-rls}
---
Sharing, groups and folders put the access decision in other relations.

## security/policy-reads/row-only

```
policy own on docs for read = row.owner == caller.user
```

Cheap to push into an index range; group membership needs
denormalising into the row.

## security/policy-reads/relations

```
policy shared on docs for read =
  exists m in reachable(caller.user) where m.folder == row.folder
fn reachable (u: Key users) -> Set Grant = fix r = (from g in grants where g.user == u select g) ...
```

Zanzibar's relation rewrites as an ordinary recursive query
(`lang/recursion/fix`). SQL Server predicates join lookup tables.
