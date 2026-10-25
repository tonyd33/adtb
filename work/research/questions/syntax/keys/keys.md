---
title: "Where keys, uniques and indexes are declared"
pick: one
options:
  on-relation:
    title: "Clauses on the relation declaration"
  separate:
    title: "Separate unique and index declarations naming the constructor set"
---
Keys and uniques belong to relation variables, over constructor sets.

## syntax/keys/on-relation

```
relation identities : Identity
  key id
  unique (gid) in Google
  unique (email) in EmailPass
  index (user) in (Google | Apple)
```

## syntax/keys/separate

```
relation identities : Identity key id
unique identities.Google (gid)
index identities.(Google | Apple) (user)
```
