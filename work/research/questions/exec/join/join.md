---
title: Where a join with a matched sum goes relative to its arms
pick: one
raised-by: [query/union-all]
options:
  distribute:
    title: Push the join into every arm
  hoist:
    title: Join once above the union when the key is a shared field
  cost:
    title: The optimizer chooses per query between both
---
`(A1 ∪ A2 ∪ A3) ⋈ U = (A1 ⋈ U) ∪ (A2 ⋈ U) ∪ (A3 ⋈ U)` holds for bags,
so both plans are correct; their cost differs.

```
from i in identities, u in users where i.user == key u
select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid }
```

## exec/join/distribute

Each arm `identities.C ⋈ users` is ordered and costed on its own heap's
statistics; a selective arm can use an index nested loop while a large
one hashes. Postgres partitionwise join applies when both sides are
partitioned alike (unverified as applicable to one side).

```
Union
  HashJoin(user) <- Scan(email_pass), Scan(user)
  IndexNL(user_pk) <- Scan(google)
```

## exec/join/hoist

`user` is a shared field of every constructor, so the union of
`(user, arm output)` rows is joined once. Needs the join key in every
arm's output.

```
HashJoin(user) <- Union(Scan(email_pass) -> arm0, Scan(google) -> arm1), Scan(user)
```

## exec/join/cost

Enumerate both per join; arms are independent once distributed, so the
search grows with the arm count times the per-arm join space.
